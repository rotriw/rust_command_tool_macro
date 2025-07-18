extern crate proc_macro;

use proc_macro::TokenStream;
use syn::Ident;
use quote::{quote, ToTokens};
use std::{fs, path::Path};
use syn::{parse_macro_input, ItemFn};

fn parse_token(item: TokenStream) -> Result<(String, Ident), syn::Error> {
    let input = item.to_string();
    let parts: Vec<&str> = input.split('\"').collect();
    let mut src = String::from("src/command");
    let mut exec_func = String::from("exec_command");
    let mut is_src = false;
    let mut is_exec_func = false;
    for part in parts.iter() {
        if part.contains("src") && !is_src {
            is_src = true;
            continue;
        }
        if part.contains("exec_func") && !is_exec_func {
            is_exec_func = true;
            continue;
        }
        if is_src && is_exec_func {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "Cannot distict the arguments",
            ));
        }
        if is_src {
            src = part.trim().to_string();
            is_src = false;
        } else if is_exec_func {
            exec_func = part.trim().to_string();
            is_exec_func = false;
        }
    }

    let exec_func = syn::Ident::new(&exec_func, proc_macro2::Span::call_site());
    Ok((src, exec_func))
}

fn generate_from_dir(
    command_dir: &str,
    ident_basic: &str,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    let path_name = Path::new(command_dir).file_name();
    let now_ident = if ident_basic.is_empty() {
        format!("{}", path_name.unwrap().to_str().unwrap())
    } else {
        format!("{}::{}", ident_basic, path_name.unwrap().to_str().unwrap())
    };
    let mut command_info = Modfile {
        command_name: String::new(),
        description: String::new(),
    };
    let mut subcommands = Vec::new();
    let mut run_functions = Vec::new();
    for entry in fs::read_dir(command_dir).expect("Failed to read command directory") {
        let entry = entry.expect("Failed to read entry");
        let path = entry.path();
        if path.is_file()
            && path.extension().unwrap_or_default() == "rs"
            && path.file_name() != Some("mod.rs".as_ref())
        {
            let content = fs::read_to_string(&path).expect("Failed to read file");
            if let Some((command, run_function)) = parse_command_and_run_from_file(
                &content,
                format!("{}::{}", now_ident, path.file_stem().unwrap().to_str().unwrap()).as_str(),
            ) {
                subcommands.push(command);
                run_functions.push(run_function);
            }
        }
        if path.is_file()
            && path.file_name().unwrap() == "mod.rs"
        {
            let content = fs::read_to_string(&path).expect("Failed to read mod.rs file");
            command_info = parse_modfile(&content);
        }
        if path.is_dir() {
            // subcommand
            let (generate_code, match_code) = generate_from_dir(path.to_str().unwrap(), now_ident.as_str());
            subcommands.push(generate_code);
            let command_name = proc_macro2::Literal::string(
                command_info.command_name.as_str(),
            );
            run_functions.push((
                quote! { #command_name },
                match_code,
            ));
        }
    }
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
        match matches.subcommand() {
            #(#match_arms,)*
            Some((_,_)) => {}
            None => { log::warn!("") }
        }
    };
    (generate_code, match_code)
}

fn parse_command_and_run_from_file(
    content: &str,
    path: &str,
) -> Option<(
    proc_macro2::TokenStream,
    (proc_macro2::TokenStream, proc_macro2::TokenStream),
)> {
    let router_line = content
        .lines()
        .find(|line| line.starts_with("//! router:"))?;
    let description_line = content
        .lines()
        .find(|line| line.starts_with("//! description:"))?;
    let command_name = router_line.trim_start_matches("//! router:").trim();
    let description = description_line
        .trim_start_matches("//! description:")
        .trim();
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
                    options.push(quote! {
                        .arg(clap::Arg::new(#arg_name).required(true).help(#arg_help_literal))
                    });
                    continue;
                } else if arg.len() == 2 {
                    let default_value = arg[1];
                    let arg_name = proc_macro2::Literal::string(arg[0]);
                    let arg_help_literal = proc_macro2::Literal::string(arg_help);
                    let default_value = proc_macro2::Literal::string(default_value);
                    options.push(quote! {
                        .arg(clap::Arg::new(#arg_name).required(true).default_value(#default_value).help(#arg_help_literal))
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
                    options.push(quote! {
                        .arg(clap::Arg::new(#arg_name).required(false).help(#arg_help_literal))
                    });
                    continue;
                } else if arg.len() == 2 {
                    let default_value = arg[1];
                    let arg_name = proc_macro2::Literal::string(arg[0]);
                    let arg_help_literal = proc_macro2::Literal::string(arg_help);
                    let default_value = proc_macro2::Literal::string(default_value);
                    options.push(quote! {
                        .arg(clap::Arg::new(#arg_name).required(false).default_value(#default_value).help(#arg_help_literal))
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
    let mut run_str = "".to_string();
    let mut is_run_line = false;
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
                    options.push(quote! {
                        .arg(clap::Arg::new(#value_name)
                            .short(#short_flag_char)
                            .long(#option_name)
                            .value_name(#value_name)
                            .help(#help_message))
                    });
                }
            }
        }
        if line.contains("pub fn run(") {
            is_run_line = true;
        }
        if is_run_line {
            run_str.push_str(line);
        }
        if line.contains(")") {
            is_run_line = false;
        }
    }
    // run_fn only save illegal char
    let run_str = run_str
        .chars()
        .filter(|c| c.is_alphabetic() || "<>(){}^_',:0123456789".contains(*c))
        .collect::<String>();
    // find run fun
    let run_fn_signature = run_str.trim().trim_start_matches("pubfnrun(");
    // then run find first ) and first }
    let run_fn_signature = run_fn_signature.split(')').next().unwrap();
    let run_fn_args: Vec<&str> = run_fn_signature.split(',').collect();
    let run_fn_args: Vec<&str> = run_fn_args.iter().filter(|arg| !arg.trim().is_empty()).map(|arg| arg.trim()).collect();
    let run_fn_call = run_fn_args.iter().map(|arg| {
        let arg_name = arg.split(':').next().unwrap().trim();
        let type_d = arg.split(':').nth(1).unwrap().trim();
        if type_d.starts_with("Option<") {
            let type_d_inner = type_d.trim_start_matches("Option<");
            let type_d_inner = type_d_inner[..type_d_inner.len() - 1].to_string();
            // check vector
            if type_d_inner.starts_with("Vec<") {
                let type_d_inner = type_d_inner.trim_start_matches("Vec<");
                let type_d_inner = type_d_inner[..type_d_inner.len() - 1].to_string();
                let type_d_ident = syn::parse_str::<syn::Type>(type_d_inner.as_str()).unwrap();
                quote! { { let _res = sub_m.get_matches::<Vec<#type_d_ident>>(#arg_name); if _res == None {
                    None
                } else {
                    Some(_res.unwrap().clone())
                } } }
            } else {
                let type_d_ident = syn::parse_str::<syn::Type>(type_d_inner.as_str()).unwrap();
                quote! { { let _res = sub_m.get_one::<#type_d_ident>(#arg_name); if _res == None {
                    None
                } else {
                    Some(_res.unwrap().clone())
                } } }
            }
        } else {
            if type_d.starts_with("Vec<") {
                let type_d_inner = type_d.trim_start_matches("Vec<");
                let type_d_inner = type_d_inner[..type_d_inner.len() - 1].to_string();
                let type_d_ident = syn::Ident::new(type_d_inner.as_str(), proc_macro2::Span::call_site());
                quote! { sub_m.get_one::<Vec<#type_d_ident>>(#arg_name).unwrap().clone() }
            } else {
                let type_d_ident = syn::Ident::new(type_d, proc_macro2::Span::call_site());
                quote! { sub_m.get_one::<#type_d_ident>(#arg_name).unwrap().clone() }
            }
        }
    });
    let func_idents = path.split("::").collect::<Vec<&str>>();
    let func_ident = func_idents.iter().map(|s| {
        syn::Ident::new(s, proc_macro2::Span::call_site())
    });
    Some((
        quote! {
            clap::Command::new(#command_name)
                .about(#description)
                #(#options)*
        },
        (
            quote! { #command_name },
            quote! {
                #(#func_ident)::*::run(#(#run_fn_call),*);
            },
        ),
    ))
}

struct Modfile {
    command_name: String,
    description: String,
}

fn parse_modfile(
    content: &str
) -> Modfile {
    let router_line = content
        .lines()
        .find(|line| line.starts_with("//! router:"))
        .unwrap();
    let description_line = content
        .lines()
        .find(|line| line.starts_with("//! description:"))
        .unwrap();
    let command_name = router_line.trim_start_matches("//! router:").trim().to_string();
    let description = description_line
        .trim_start_matches("//! description:")
        .trim()
        .to_string();
    Modfile {
        command_name,
        description,
    }
}

#[proc_macro]
pub fn generate_commands(item: TokenStream) -> TokenStream {
    let (command_dir, execfunc) = parse_token(item).unwrap();
    let (generate_code, match_code) = generate_from_dir(command_dir.as_str(), "");
    let expanded = quote! {
        pub fn build_cli() -> clap::Command {
            #generate_code
        }
        pub fn #execfunc() {
            let matches = build_cli().get_matches();
            #match_code
        }
    };

    TokenStream::from(expanded)
}