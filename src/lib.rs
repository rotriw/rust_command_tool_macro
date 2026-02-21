use proc_macro::TokenStream;
use quote::quote;
use std::{collections::HashMap, fs, path::Path};
use syn::{parse::Parse, parse::ParseStream, Attribute, Ident, ItemFn, LitStr, Result, Token, Type};

struct MacroArgs {
    src: String,
    exec_func: Ident,
}

impl Parse for MacroArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut src: Option<String> = None;
        let mut exec_func: Option<Ident> = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            let _eq: Token![=] = input.parse()?;
            if key == "src" {
                let value: LitStr = input.parse()?;
                src = Some(value.value());
            } else if key == "exec_func" {
                let value: LitStr = input.parse()?;
                exec_func = Some(Ident::new(&value.value(), value.span()));
            } else if key == "help_message" {
                let _value: LitStr = input.parse()?;
            } else {
                return Err(syn::Error::new_spanned(key, "Unknown argument"));
            }

            if input.peek(Token![,]) {
                let _comma: Token![,] = input.parse()?;
            }
        }

        Ok(MacroArgs {
            src: src.unwrap_or_else(|| "src/command".to_string()),
            exec_func: exec_func.unwrap_or_else(|| Ident::new("exec_command", proc_macro2::Span::call_site())),
        })
    }
}

fn parse_token(item: TokenStream) -> Result<(String, Ident)> {
    let args = syn::parse::<MacroArgs>(item)?;
    Ok((args.src, args.exec_func))
}

struct GeneratedDir {
    generate_code: proc_macro2::TokenStream,
    match_code: proc_macro2::TokenStream,
    command_name: String,
}

fn generate_from_dir(
    command_dir: &str,
    ident_basic: &str,
    matches_ident: &proc_macro2::TokenStream,
) -> Result<GeneratedDir> {
    let path_name = Path::new(command_dir)
        .file_name()
        .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "Invalid command directory"))?;
    let path_name = path_name
        .to_str()
        .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "Invalid command directory name"))?;
    let now_ident = if ident_basic.is_empty() {
        path_name.to_string()
    } else {
        format!("{}::{}", ident_basic, path_name)
    };
    let mut command_info: Option<Modfile> = None;
    let mut subcommands = Vec::new();
    let mut run_functions = Vec::new();
    for entry in fs::read_dir(command_dir)
        .map_err(|e| syn::Error::new(proc_macro2::Span::call_site(), format!("Failed to read command directory: {e}")))?
    {
        let entry = entry.map_err(|e| {
            syn::Error::new(proc_macro2::Span::call_site(), format!("Failed to read directory entry: {e}"))
        })?;
        let path = entry.path();
        if path.is_file()
            && path.extension().unwrap_or_default() == "rs"
            && path.file_name() != Some("mod.rs".as_ref())
        {
            let content = fs::read_to_string(&path).map_err(|e| {
                syn::Error::new(proc_macro2::Span::call_site(), format!("Failed to read file: {e}"))
            })?;
            let file_stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "Invalid file name"))?;
            if let Some((command, run_function)) = parse_command_and_run_from_file(
                &content,
                format!("{}::{}", now_ident, file_stem).as_str(),
            )? {
                subcommands.push(command);
                run_functions.push(run_function);
            }
        }
        if path.is_file() && path.file_name() == Some("mod.rs".as_ref()) {
            let content = fs::read_to_string(&path).map_err(|e| {
                syn::Error::new(proc_macro2::Span::call_site(), format!("Failed to read mod.rs file: {e}"))
            })?;
            command_info = Some(parse_modfile(&content)?);
        }
        if path.is_dir() {
            let child = generate_from_dir(
                path.to_str().ok_or_else(|| {
                    syn::Error::new(proc_macro2::Span::call_site(), "Invalid subcommand directory")
                })?,
                now_ident.as_str(),
                &quote! { sub_m },
            )?;
            let command_name_literal = proc_macro2::Literal::string(child.command_name.as_str());
            subcommands.push(child.generate_code);
            run_functions.push((quote! { #command_name_literal }, child.match_code));
        }
    }

    let command_info = command_info.ok_or_else(|| {
        syn::Error::new(proc_macro2::Span::call_site(), "Missing mod.rs with router/description")
    })?;
    let subcommands_code = subcommands.into_iter().map(|cmd| {
        quote! {
            .subcommand(#cmd)
        }
    });

    let match_arms = run_functions
        .into_iter()
        .map(|(cmd_name, run_function_call)| {
            quote! {
                Some((#cmd_name, sub_m)) => {
                    #run_function_call
                }
            }
        });
    let command_name = proc_macro2::Literal::string(command_info.command_name.as_str());
    let description = proc_macro2::Literal::string(command_info.description.as_str());
    let generate_code = quote! {
        clap::Command::new(#command_name)
        .arg_required_else_help(true)
        .about(#description)
        #(#subcommands_code)*
    };

    let match_code = quote! {
        match #matches_ident.subcommand() {
            #(#match_arms,)*
            Some((_,_)) => {}
            None => { log::warn!("") }
        }
    };
    Ok(GeneratedDir {
        generate_code,
        match_code,
        command_name: command_info.command_name,
    })
}

fn parse_command_and_run_from_file(
    content: &str,
    path: &str,
) -> Result<Option<(
    proc_macro2::TokenStream,
    (proc_macro2::TokenStream, proc_macro2::TokenStream),
)>> {
    let router_line = content
        .lines()
        .find(|line| line.starts_with("//! router:"));
    let description_line = content
        .lines()
        .find(|line| line.starts_with("//! description:"));
    if router_line.is_none() || description_line.is_none() {
        return Ok(None);
    }
    let router_line = router_line.unwrap();
    let description_line = description_line.unwrap();
    let command_name = router_line.trim_start_matches("//! router:").trim();
    let description = description_line
        .trim_start_matches("//! description:")
        .trim();
    let file_ast = syn::parse_file(content)?;
    let run_fn = find_run_fn(&file_ast.items, path)?;
    let (run_fn_name, run_args, arg_types) = extract_fn_signature(run_fn)?;
    let args = content.lines().find(|line| line.starts_with("//! args:"));
    let mut options = Vec::new();
    // handle args.
    if let Some(args) = args {
        let args = args.trim_start_matches("//! args:").trim();
        let args = args.split_whitespace().collect::<Vec<&str>>();
        for arg in args {
            if arg.starts_with('<') && (arg.ends_with('>') || arg.ends_with(')')) {
                let arg = arg.trim_start_matches('<').trim_end_matches('>');
                //parse arg help.
                //<abc:default_value>(help)
                //parse default_value and arg.
                let arg_help = arg.split('(').nth(1).unwrap_or("").trim_end_matches(')');
                let arg = arg.split('(').collect::<Vec<&str>>()[0]
                    .trim_start_matches('<')
                    .trim_end_matches('>');
                let arg = arg.split(':').collect::<Vec<&str>>();
                if arg.len() == 1 {
                    let arg_name = proc_macro2::Literal::string(arg[0]);
                    let arg_help_literal = proc_macro2::Literal::string(arg_help);
                    let arg_options = arg_types.get(arg[0]).copied().unwrap_or(ArgType::PLAIN);
                    let num_args = arg_options.num_args_tokens();
                    options.push(quote! {
                        .arg(clap::Arg::new(#arg_name).required(true).help(#arg_help_literal)#num_args)
                    });
                    continue;
                } else if arg.len() == 2 {
                    let default_value = arg[1];
                    let arg_name = proc_macro2::Literal::string(arg[0]);
                    let arg_help_literal = proc_macro2::Literal::string(arg_help);
                    let default_value = proc_macro2::Literal::string(default_value);
                    let arg_options = arg_types.get(arg[0]).copied().unwrap_or(ArgType::PLAIN);
                    let num_args = arg_options.num_args_tokens();
                    options.push(quote! {
                        .arg(clap::Arg::new(#arg_name).required(true).default_value(#default_value).help(#arg_help_literal)#num_args)
                    });
                    continue;
                }
            } else if arg.starts_with('[') && (arg.ends_with(']') || arg.ends_with(')')) {
                let arg_help = arg.split('(').nth(1).unwrap_or("").trim_end_matches(')');
                let arg = arg.split('(').collect::<Vec<&str>>()[0]
                    .trim_start_matches('[')
                    .trim_end_matches(']');
                let arg = arg.split(':').collect::<Vec<&str>>();
                if arg.len() == 1 {
                    let arg_name = proc_macro2::Literal::string(arg[0]);
                    let arg_help_literal = proc_macro2::Literal::string(arg_help);
                    let arg_options = arg_types.get(arg[0]).copied().unwrap_or(ArgType::PLAIN);
                    let num_args = arg_options.num_args_tokens();
                    options.push(quote! {
                        .arg(clap::Arg::new(#arg_name).required(false).help(#arg_help_literal)#num_args)
                    });
                    continue;
                } else if arg.len() == 2 {
                    let default_value = arg[1];
                    let arg_name = proc_macro2::Literal::string(arg[0]);
                    let arg_help_literal = proc_macro2::Literal::string(arg_help);
                    let default_value = proc_macro2::Literal::string(default_value);
                    let arg_options = arg_types.get(arg[0]).copied().unwrap_or(ArgType::PLAIN);
                    let num_args = arg_options.num_args_tokens();
                    options.push(quote! {
                        .arg(clap::Arg::new(#arg_name).required(false).default_value(#default_value).help(#arg_help_literal)#num_args)
                    });
                    continue;
                }
            }
        }
    }
    // find log_level required flag
    let log_level_flag = content
        .lines()
        .find(|line| line.starts_with("//! log_level required"));
    if let Some(_log_level_flag) = log_level_flag {
        options.push(quote! {
            .arg(clap::Arg::new("log_level").long("log_level").value_name("log_level").default_value("info").help("Set Log level(trace, debug, info, warn, error, off)"))
        });
    }
    for line in content.lines() {
        if line.starts_with("//! --") {
            let option_def = line.trim_start_matches("//! ").trim();
            let option_def = option_def.replace("\\,", "PLACEHOLDER");
            let parts: Vec<&str> = option_def.split(',').collect();
            if parts.len() == 2 {
                let option_args = parts[0].split_whitespace().collect::<Vec<&str>>();
                let help_message = parts[1].trim().replace("PLACEHOLDER", ",");
                if option_args.len() >= 3 {
                    let option_name = option_args[0].trim_start_matches("--");
                    let short_flag = option_args[1].trim_start_matches('-');
                    let value_name = option_args[2].trim_start_matches('<').trim_end_matches('>');
                    let short_flag_char = short_flag.chars().next().unwrap();
                    let arg_options = arg_types.get(value_name).copied().unwrap_or(ArgType::PLAIN);
                    let num_args = arg_options.num_args_tokens();
                    options.push(quote! {
                        .arg(clap::Arg::new(#value_name)
                            .short(#short_flag_char)
                            .long(#option_name)
                            .value_name(#value_name)
                            .help(#help_message)#num_args)
                    });
                }
            }
        }
    }
    let run_fn_call = run_args.iter().map(|(arg_name, arg_type)| {
        let arg_name_literal = proc_macro2::Literal::string(arg_name.as_str());
        let inner_type = &arg_type.inner;
        if arg_type.is_option {
            if arg_type.is_vec {
                quote! {
                    sub_m.get_many::<#inner_type>(#arg_name_literal)
                        .map(|vals| vals.cloned().collect::<Vec<#inner_type>>())
                }
            } else {
                quote! { sub_m.get_one::<#inner_type>(#arg_name_literal).cloned() }
            }
        } else if arg_type.is_vec {
            quote! {
                sub_m.get_many::<#inner_type>(#arg_name_literal)
                    .expect("missing required argument")
                    .cloned()
                    .collect::<Vec<#inner_type>>()
            }
        } else {
            quote! { sub_m.get_one::<#inner_type>(#arg_name_literal).expect("missing required argument").clone() }
        }
    });
    let func_idents = path.split("::").collect::<Vec<&str>>();
    let func_ident = func_idents.iter().map(|s| {
        syn::Ident::new(s, proc_macro2::Span::call_site())
    });
    Ok(Some((
        quote! {
            clap::Command::new(#command_name)
                .about(#description)
                #(#options)*
        },
        (
            quote! { #command_name },
            quote! {
                #(#func_ident)::*::#run_fn_name(#(#run_fn_call),*);
            },
        ),
    )))
}

struct Modfile {
    command_name: String,
    description: String,
}

fn parse_modfile(content: &str) -> Result<Modfile> {
    let router_line = content
        .lines()
        .find(|line| line.starts_with("//! router:"))
        .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "Missing //! router:"))?;
    let description_line = content
        .lines()
        .find(|line| line.starts_with("//! description:"))
        .ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), "Missing //! description:"))?;
    let command_name = router_line.trim_start_matches("//! router:").trim().to_string();
    let description = description_line
        .trim_start_matches("//! description:")
        .trim()
        .to_string();
    Ok(Modfile {
        command_name,
        description,
    })
}

#[proc_macro]
pub fn generate_commands(item: TokenStream) -> TokenStream {
    let result: Result<proc_macro2::TokenStream> = (|| {
        let (command_dir, execfunc) = parse_token(item)?;
        let generated = generate_from_dir(command_dir.as_str(), "", &quote! { matches })?;
        let GeneratedDir { generate_code, match_code, .. } = generated;
        let expanded = quote! {
            pub fn build_cli() -> clap::Command {
                #generate_code
            }
            pub fn #execfunc() {
                let matches = build_cli().get_matches();
                #match_code
            }
        };
        Ok(expanded)
    })();

    match result {
        Ok(tokens) => TokenStream::from(tokens),
        Err(err) => TokenStream::from(err.to_compile_error()),
    }
}

#[proc_macro_attribute]
pub fn run(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[derive(Clone, Copy)]
struct ArgType {
    is_option: bool,
    is_vec: bool,
}

impl ArgType {
    const PLAIN: ArgType = ArgType { is_option: false, is_vec: false };
    fn num_args_tokens(self) -> proc_macro2::TokenStream {
        if self.is_vec {
            quote! { .num_args(1..) }
        } else {
            quote! {}
        }
    }
}

fn find_run_fn<'a>(items: &'a [syn::Item], path: &str) -> Result<&'a ItemFn> {
    let mut run_fn: Option<&ItemFn> = None;
    for item in items {
        if let syn::Item::Fn(func) = item {
            if has_run_attr(&func.attrs) {
                if run_fn.is_some() {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!("Multiple #[run] functions found in {path}"),
                    ));
                }
                run_fn = Some(func);
            }
        }
    }
    run_fn.ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("Missing #[run] function in {path}"),
        )
    })
}

fn has_run_attr(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("run"))
}

fn extract_fn_signature(func: &ItemFn) -> Result<(Ident, Vec<(String, ArgTypeWithInner)>, HashMap<String, ArgType>)> {
    let mut args = Vec::new();
    let mut arg_types = HashMap::new();

    for input in func.sig.inputs.iter() {
        let typed = match input {
            syn::FnArg::Typed(typed) => typed,
            syn::FnArg::Receiver(_) => {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "#[run] function cannot take self parameter",
                ))
            }
        };
        let name = match typed.pat.as_ref() {
            syn::Pat::Ident(pat_ident) => pat_ident.ident.to_string(),
            _ => {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "#[run] argument must be an identifier",
                ))
            }
        };
        let (arg_type, inner) = normalize_type(&typed.ty)?;
        args.push((name.clone(), ArgTypeWithInner { is_option: arg_type.is_option, is_vec: arg_type.is_vec, inner: inner.clone() }));
        arg_types.insert(name, arg_type);
    }

    Ok((func.sig.ident.clone(), args, arg_types))
}

#[derive(Clone)]
struct ArgTypeWithInner {
    is_option: bool,
    is_vec: bool,
    inner: Type,
}

fn normalize_type(ty: &Type) -> Result<(ArgType, Type)> {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Option" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first() {
                        let (inner_arg_type, inner_type) = normalize_vec_type(inner_ty)?;
                        return Ok((
                            ArgType { is_option: true, is_vec: inner_arg_type.is_vec },
                            inner_type,
                        ));
                    }
                }
            }
        }
    }

    let (inner_arg_type, inner_type) = normalize_vec_type(ty)?;
    Ok((ArgType { is_option: false, is_vec: inner_arg_type.is_vec }, inner_type))
}

fn normalize_vec_type(ty: &Type) -> Result<(ArgType, Type)> {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Vec" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first() {
                        return Ok((ArgType { is_option: false, is_vec: true }, inner_ty.clone()));
                    }
                }
            }
        }
    }
    Ok((ArgType::PLAIN, ty.clone()))
}