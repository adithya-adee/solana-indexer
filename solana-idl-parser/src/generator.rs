use crate::model::*;
use anyhow::Result;
use proc_macro2::Span;
use quote::quote;
use syn::Ident;

/// Generation mode for IDL parser
#[derive(Debug, Clone, Copy)]
pub enum GenerationMode {
    /// Generate Anchor-compatible types (uses AnchorSerialize, AnchorDeserialize, etc.)
    Anchor,
    /// Generate SDK-compatible types (uses BorshSerialize, BorshDeserialize, EventDiscriminator)
    Sdk,
}

/// Generate Rust types from IDL (Anchor mode - default)
pub fn generate_types(idl: &Idl) -> Result<String> {
    generate_types_with_mode(idl, GenerationMode::Anchor)
}

/// Generate Rust types from IDL with specified mode
pub fn generate_types_with_mode(idl: &Idl, mode: GenerationMode) -> Result<String> {
    match mode {
        GenerationMode::Anchor => generate_anchor_types(idl),
        GenerationMode::Sdk => generate_sdk_types(idl),
    }
}

/// Generate Anchor-compatible types
fn generate_anchor_types(idl: &Idl) -> Result<String> {
    let mut code = quote! {
        use anchor_lang::prelude::*;
    };

    for ty in &idl.types {
        let type_name = &ty.name;
        let type_name_ident = Ident::new(type_name, Span::call_site());

        let fields = ty.ty.fields.iter().map(|field| {
            let field_name = &field.name;
            let field_name_ident = Ident::new(field_name, Span::call_site());
            let field_ty = idl_type_to_rust_type(&field.ty, GenerationMode::Anchor);
            quote! {
                pub #field_name_ident: #field_ty,
            }
        });

        let type_struct = quote! {
            #[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
            pub struct #type_name_ident {
                #(#fields)*
            }
        };

        code.extend(type_struct);
    }

    for event in &idl.events {
        let event_name = &event.name;
        let event_name_ident = Ident::new(event_name, Span::call_site());

        let fields = event.fields.iter().map(|field| {
            let field_name = &field.name;
            let field_name_ident = Ident::new(field_name, Span::call_site());
            let field_ty = idl_type_to_rust_type(&field.ty, GenerationMode::Anchor);
            quote! {
                pub #field_name_ident: #field_ty,
            }
        });

        let event_struct = quote! {
            #[event]
            pub struct #event_name_ident {
                #(#fields)*
            }
        };

        code.extend(event_struct);
    }

    let error_codes = idl.errors.iter().map(|error| {
        let error_name = &error.name;
        let error_name_ident = Ident::new(error_name, Span::call_site());
        let error_msg = &error.msg;
        quote! {
            #[msg(#error_msg)]
            #error_name_ident,
        }
    });

    let error_enum = quote! {
        #[error_code]
        pub enum SolraiserError {
            #(#error_codes)*
        }
    };

    code.extend(error_enum);

    Ok(code.to_string())
}

/// Generate SDK-compatible types (Borsh + EventDiscriminator)
fn generate_sdk_types(idl: &Idl) -> Result<String> {
    let mut code = quote! {
        use borsh::{BorshDeserialize, BorshSerialize};
        use solana_sdk::pubkey::Pubkey;
        use sha2::{Digest, Sha256};
        use solana_indexer_sdk::EventDiscriminator;
    };

    // Generate types
    for ty in &idl.types {
        let type_name = &ty.name;
        let type_name_ident = Ident::new(type_name, Span::call_site());

        let fields = ty.ty.fields.iter().map(|field| {
            let field_name = &field.name;
            let field_name_ident = Ident::new(field_name, Span::call_site());
            let field_ty = idl_type_to_rust_type(&field.ty, GenerationMode::Sdk);
            quote! {
                pub #field_name_ident: #field_ty,
            }
        });

        let type_struct = quote! {
            #[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
            pub struct #type_name_ident {
                #(#fields)*
            }
        };

        code.extend(type_struct);
    }

    // Generate events with EventDiscriminator
    for event in &idl.events {
        let event_name = &event.name;
        let event_name_ident = Ident::new(event_name, Span::call_site());

        let fields = event.fields.iter().map(|field| {
            let field_name = &field.name;
            let field_name_ident = Ident::new(field_name, Span::call_site());
            let field_ty = idl_type_to_rust_type(&field.ty, GenerationMode::Sdk);
            quote! {
                pub #field_name_ident: #field_ty,
            }
        });

        // Calculate discriminator for the event
        let discriminator_preimage = format!("event:{}", event_name);
        let discriminator_bytes = calculate_discriminator_bytes(&discriminator_preimage);
        let discriminator_lit = quote! { [#(#discriminator_bytes),*] };

        let event_struct = quote! {
            #[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
            pub struct #event_name_ident {
                #(#fields)*
            }

            impl #event_name_ident {
                /// Returns the event discriminator for `#event_name`.
                #[must_use]
                pub fn discriminator() -> [u8; 8] {
                    #discriminator_lit
                }
            }

            impl EventDiscriminator for #event_name_ident {
                fn discriminator() -> [u8; 8] {
                    Self::discriminator()
                }
            }
        };

        code.extend(event_struct);
    }

    // Generate error enum (simplified for SDK - no Anchor attributes)
    if !idl.errors.is_empty() {
        let error_codes = idl.errors.iter().map(|error| {
            let error_name = &error.name;
            let error_name_ident = Ident::new(error_name, Span::call_site());
            let error_code = error.code;
            let error_msg = &error.msg;
            let error_msg_str = format!(" // {}", error_msg);
            quote! {
                #error_name_ident = #error_code,#error_msg_str
            }
        });

        let error_enum = quote! {
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub enum ProgramError {
                #(#error_codes)*
            }
        };

        code.extend(error_enum);
    }

    Ok(code.to_string())
}

/// Calculate discriminator bytes from a preimage string
fn calculate_discriminator_bytes(preimage: &str) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(preimage.as_bytes());
    hash[..8].to_vec()
}

fn idl_type_to_rust_type(
    idl_type: &IdlTypeDefinition,
    mode: GenerationMode,
) -> proc_macro2::TokenStream {
    match idl_type {
        IdlTypeDefinition::Simple(s) => match s.as_str() {
            "bool" => quote! { bool },
            "u8" => quote! { u8 },
            "i8" => quote! { i8 },
            "u16" => quote! { u16 },
            "i16" => quote! { i16 },
            "u32" => quote! { u32 },
            "i32" => quote! { i32 },
            "u64" => quote! { u64 },
            "i64" => quote! { i64 },
            "u128" => quote! { u128 },
            "i128" => quote! { i128 },
            "bytes" => quote! { Vec<u8> },
            "string" => quote! { String },
            "publicKey" => match mode {
                GenerationMode::Anchor => quote! { Pubkey },
                GenerationMode::Sdk => quote! { Pubkey },
            },
            _ => {
                let ident = Ident::new(s, Span::call_site());
                quote! { #ident }
            }
        },
        IdlTypeDefinition::Complex(c) => match c {
            IdlTypeDefinitionComplex::Defined(s) => {
                let ident = Ident::new(s, Span::call_site());
                quote! { #ident }
            }
            IdlTypeDefinitionComplex::Option(t) => {
                let inner = idl_type_to_rust_type(t, mode);
                quote! { Option<#inner> }
            }
            IdlTypeDefinitionComplex::Vec(t) => {
                let inner = idl_type_to_rust_type(t, mode);
                quote! { Vec<#inner> }
            }
            IdlTypeDefinitionComplex::Array(a) => {
                if a.len() == 2 {
                    let ty = &a[0];
                    let size = &a[1];
                    if let (Some(ty_str), Some(size_int)) = (ty.as_str(), size.as_u64()) {
                        let ty_ident = Ident::new(ty_str, Span::call_site());
                        let size_lit = syn::LitInt::new(&size_int.to_string(), Span::call_site());
                        quote! { [#ty_ident; #size_lit] }
                    } else {
                        quote! { Vec<u8> }
                    }
                } else {
                    quote! { Vec<u8> }
                }
            }
        },
    }
}
