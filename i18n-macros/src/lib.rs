//! Compile translated format strings with Rust's own format checker. Keeping
//! literal spans preserves named captures, formatting precision and argument types.
use proc_macro::{Group, Literal, TokenStream, TokenTree};

fn substitute(template: &str, replacements: &[(&str, TokenStream)]) -> TokenStream {
    fn walk(input: TokenStream, replacements: &[(&str, TokenStream)]) -> TokenStream {
        input
            .into_iter()
            .flat_map(|token| match token {
                TokenTree::Ident(ref id)
                    if replacements.iter().any(|(key, _)| id.to_string() == *key) =>
                {
                    replacements
                        .iter()
                        .find(|(key, _)| id.to_string() == *key)
                        .unwrap()
                        .1
                        .clone()
                }
                TokenTree::Group(group) => {
                    let mut next =
                        Group::new(group.delimiter(), walk(group.stream(), replacements));
                    next.set_span(group.span());
                    TokenStream::from(TokenTree::Group(next))
                }
                token => TokenStream::from(token),
            })
            .collect()
    }
    walk(template.parse().unwrap(), replacements)
}

fn translated(input: TokenStream) -> TokenStream {
    let mut tokens = input.clone().into_iter();
    let Some(TokenTree::Literal(literal)) = tokens.next() else {
        return substitute("std::format!(__args)", &[("__args", input)]);
    };
    let source: Option<String> = serde_json::from_str(&literal.to_string()).ok();
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("../../locales/messages.json")).unwrap();
    let entry = source.as_ref().and_then(|source| catalog.get(source));
    let Some(entry) = entry else {
        return substitute("std::format!(__args)", &[("__args", input)]);
    };
    let rest: TokenStream = tokens.collect();
    let mut branches = TokenStream::new();
    for language in ["de", "fr", "es", "en"] {
        let value = if language == "de" {
            source.as_deref().unwrap()
        } else {
            entry[language]
                .as_str()
                .expect("complete translation catalog")
        };
        let mut message = Literal::string(value);
        message.set_span(literal.span());
        branches.extend(substitute(
            "__lang => std::format!(__message __rest),",
            &[
                (
                    "__lang",
                    TokenTree::Literal(Literal::string(language)).into(),
                ),
                ("__message", TokenTree::Literal(message).into()),
                ("__rest", rest.clone()),
            ],
        ));
    }
    // language() always returns one of the four supported codes.
    branches.extend("_ => unreachable!(),".parse::<TokenStream>().unwrap());
    substitute(
        "match crate::i18n::language() { __branches }",
        &[("__branches", branches)],
    )
}

#[proc_macro]
pub fn format(input: TokenStream) -> TokenStream {
    translated(input)
}
#[proc_macro]
pub fn println(input: TokenStream) -> TokenStream {
    substitute(
        "std::println!(\"{}\", __value)",
        &[("__value", translated(input))],
    )
}
#[proc_macro]
pub fn eprintln(input: TokenStream) -> TokenStream {
    substitute(
        "std::eprintln!(\"{}\", __value)",
        &[("__value", translated(input))],
    )
}
#[proc_macro]
pub fn anyhow(input: TokenStream) -> TokenStream {
    if matches!(
        input.clone().into_iter().next(),
        Some(TokenTree::Literal(_))
    ) {
        substitute(
            "anyhow::anyhow!(\"{}\", __value)",
            &[("__value", translated(input))],
        )
    } else {
        substitute("anyhow::anyhow!(__args)", &[("__args", input)])
    }
}
#[proc_macro]
pub fn bail(input: TokenStream) -> TokenStream {
    substitute(
        "return Err(crate::i18n::anyhow!(__args))",
        &[("__args", input)],
    )
}
#[proc_macro]
pub fn ensure(input: TokenStream) -> TokenStream {
    let mut tokens = input.into_iter();
    let mut condition = TokenStream::new();
    for token in tokens.by_ref() {
        if matches!(&token, TokenTree::Punct(p) if p.as_char() == ',') {
            break;
        }
        condition.extend([token]);
    }
    let rest: TokenStream = tokens.collect();
    substitute(
        "if !(__condition) { return Err(crate::i18n::anyhow!(__rest)); }",
        &[("__condition", condition), ("__rest", rest)],
    )
}
