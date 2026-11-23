mod versioned_struct;

use proc_macro::TokenStream;
use quote::quote;
use syn::{spanned::Spanned, Lit};

#[proc_macro_attribute]
pub fn versioned(attr: TokenStream, item: TokenStream) -> TokenStream {
	versioned_struct::versioned(attr.into(), item.into()).into()
}

fn extract_args(a: &syn::FnArg) -> &syn::PatType {
	match a {
		syn::FnArg::Typed(p) => p,
		_ => panic!("Not supported on types with `self`!")
	}
}

#[proc_macro_attribute]
pub fn init(attr: TokenStream, item: TokenStream) -> TokenStream {
	let init_type = syn::parse_macro_input!(attr as syn::Ident);
	let func = syn::parse_macro_input!(item as syn::ItemFn);
	let func_name = &func.sig.ident;

	let func_type = match init_type.to_string().as_str() {
		"full" => quote! { auxtools::FullInitFunc },
		"partial" => quote! { auxtools::PartialInitFunc },
		_ => return syn::Error::new(init_type.span(), "invalid init type").to_compile_error().into()
	};

	let inventory_define = quote! {
		auxtools::inventory::submit!(
			#func_type(#func_name)
		);
	};

	let code = quote! {
		#func
		#inventory_define
	};

	code.into()
}

#[proc_macro_attribute]
pub fn runtime_handler(_attr: TokenStream, item: TokenStream) -> TokenStream {
	let func = syn::parse_macro_input!(item as syn::ItemFn);
	let func_name = &func.sig.ident;

	let inventory_define = quote! {
		auxtools::inventory::submit!(
			auxtools::RuntimeErrorHook(#func_name)
		);
	};

	let code = quote! {
		#func
		#inventory_define
	};

	code.into()
}

#[proc_macro_attribute]
pub fn shutdown(_: TokenStream, item: TokenStream) -> TokenStream {
	let func = syn::parse_macro_input!(item as syn::ItemFn);
	let func_name = &func.sig.ident;

	let inventory_define = quote! {
		auxtools::inventory::submit!(
			auxtools::PartialShutdownFunc(#func_name)
		);
	};

	let code = quote! {
		#func
		#inventory_define
	};

	code.into()
}

#[proc_macro_attribute]
pub fn full_shutdown(_: TokenStream, item: TokenStream) -> TokenStream {
	let func = syn::parse_macro_input!(item as syn::ItemFn);
	let func_name = &func.sig.ident;

	let inventory_define = quote! {
		auxtools::inventory::submit!(
			#![crate = auxtools]
			auxtools::FullShutdownFunc(#func_name)
		);
	};

	let code = quote! {
		#func
		#inventory_define
	};

	code.into()
}

/// Sets whether auxtools pins its DLL on Windows, which keeps it loaded until
/// the process ends and its file locked while Dream Daemon runs. A library that
/// unpins it must have no spawned threads running when DM calls
/// `auxtools_full_shutdown`, or Dream Daemon crashes.
#[proc_macro]
pub fn pin_dll(attr: TokenStream) -> TokenStream {
	let flag = syn::parse_macro_input!(attr as syn::LitBool);
	let code = quote! {
		use std::sync::atomic::{AtomicBool, Ordering};

		#[auxtools::ctor::ctor]
		#[cfg(windows)]
		fn set_pin_dll() {
			auxtools::PIN_DLL.store(#flag, Ordering::Relaxed);
		}
	};

	code.into()
}

fn is_value(path: &syn::TypePath) -> bool {
	path.qself.is_none()
		&& path
			.path
			.segments
			.last()
			.is_some_and(|last| last.ident == "Value" && last.arguments.is_none())
}

/// Defines a proc hook. Given a proc path, the hook is installed at init.
/// Each argument is a `Value` or a `&Value`, in the proc's order. Use `_` for
/// an argument the hook doesn't use.
///
/// ```ignore
/// #[hook("/mob/proc/on_honked")]
/// fn on_honked(honker: Value) {
///     honker.call("laugh", &[])?;
///     Ok(Value::NULL)
/// }
/// ```
#[proc_macro_attribute]
pub fn hook(attr: TokenStream, item: TokenStream) -> TokenStream {
	let input = syn::parse_macro_input!(item as syn::ItemFn);
	let proc = syn::parse_macro_input!(attr as Option<syn::Lit>);
	let func_name = &input.sig.ident;
	let args = &input.sig.inputs;
	let args_len = args.len();

	match &input.sig.output {
		syn::ReturnType::Default => {}

		syn::ReturnType::Type(_, ty) => {
			return syn::Error::new(ty.span(), "Do not specify the return value of hooks")
				.to_compile_error()
				.into()
		}
	}

	let cthook_prelude = match proc {
		Some(Lit::Str(p)) => {
			quote! {
				auxtools::inventory::submit!({
					auxtools::CompileTimeHook{ proc_path: #p, hook: #func_name }
				});
			}
		}
		Some(other_literal) => {
			return syn::Error::new(other_literal.span(), "Hook attributes must be a string literal")
				.to_compile_error()
				.into()
		}
		None => quote! {}
	};
	let signature = quote! {
		fn #func_name(
			src: &auxtools::Value,
			usr: &auxtools::Value,
			mut args: Vec<auxtools::Value>,
		) -> auxtools::DMResult
	};

	let body = &input.block;
	let mut bindings = vec![];
	for (index, arg) in args.iter().map(extract_args).enumerate() {
		let value = match &*arg.ty {
			syn::Type::Path(path) if is_value(path) => quote! { args[#index].clone() },
			syn::Type::Reference(reference)
				if reference.mutability.is_none() && matches!(&*reference.elem, syn::Type::Path(path) if is_value(path)) =>
			{
				quote! { &args[#index] }
			}
			other => {
				return syn::Error::new(other.span(), "a hook argument must be `Value` or `&Value`")
					.to_compile_error()
					.into()
			}
		};
		let (pat, ty) = (&arg.pat, &arg.ty);
		match &**pat {
			syn::Pat::Wild(_) => {}
			syn::Pat::Ident(ident) if ident.subpat.is_none() => bindings.push(quote! { let #pat: #ty = #value; }),
			other => {
				return syn::Error::new(other.span(), "a hook argument must be a name or `_`")
					.to_compile_error()
					.into()
			}
		}
	}
	let result = quote! {
		#cthook_prelude
		#signature {
			if #args_len > args.len() {
				for i in 0..#args_len - args.len() {
					args.push(auxtools::Value::NULL)
				}
			}
			#(#bindings)*
			#body
		}
	};
	result.into()
}
