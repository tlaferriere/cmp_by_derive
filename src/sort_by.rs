use proc_macro2::TokenStream;
use quote::{quote_spanned, ToTokens};

use syn::{
    self, spanned::Spanned, Attribute, Data, DataStruct, DeriveInput, Error, Expr, ExprLit, Fields,
    FieldsNamed, Lit, Meta, NestedMeta,
};

const HELP_SORTBY: &str = r#"SortBy: invalid sort_by attribute, expected list form i.e #[sort_by(attr1, attr2, methodcall())]"#;

pub fn impl_sort_by_derive(input: DeriveInput) -> TokenStream {
    let input_span = input.span();
    let struct_name = input.ident.clone();

    let mut sortable_expressions = vec![];

    for attr in input
        .attrs
        .iter()
        .filter(|i| i.path.get_ident().map(|i| i == "sort_by") == Some(true))
    {
        match parse_outer(attr) {
            Ok(mut vec) => sortable_expressions.append(&mut vec),
            _ => {
                return Error::new(attr.span(), HELP_SORTBY).into_compile_error();
            }
        }
    }

    match input.data {
        Data::Struct(DataStruct {
            fields: Fields::Named(fields),
            ..
        }) => match parse_fields(fields) {
            Ok(mut result) => sortable_expressions.append(&mut result),
            Err(e) => return e.into_compile_error(),
        },
        Data::Enum(_) => (),
        _ => {
            return Error::new(
                input_span,
                r#"SortBy: expected an enum or a struct with named fields"#,
            )
            .into_compile_error();
        }
    };

    let mut iter_sort_expressions = sortable_expressions.iter();
    let ord_statement = if let Some(sort_expression) = iter_sort_expressions.next() {
        quote_spanned! { sort_expression.span() =>
            core::cmp::Ord::cmp(&self.#sort_expression, &other.#sort_expression)
        }
    } else {
        return Error::new(
            input_span,
            r#"SortBy: no field to sort on. Mark fields to sort on with #[sort_by]"#,
        )
        .into_compile_error();
    };

    let ord_statement = iter_sort_expressions.fold(ord_statement, |ord_statement, field_name| {
        syn::parse_quote_spanned! {field_name.span() =>
            #ord_statement.then_with(|| self.#field_name.cmp(&other.#field_name))
        }
    });

    let hash_expressions: Vec<Expr> = sortable_expressions
        .iter()
        .map(|expr| syn::parse_quote_spanned!(expr.span() => self.#expr.hash(state)))
        .collect();

    quote_spanned! {input_span =>
        impl std::hash::Hash for #struct_name {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                #(#hash_expressions);*;
            }
        }

        impl core::cmp::Eq for #struct_name {}

        impl core::cmp::PartialEq<Self> for #struct_name {
            fn eq(&self, other: &Self) -> bool {
                self.cmp(other).is_eq()
            }
        }

        impl core::cmp::PartialOrd<Self> for #struct_name {
            fn partial_cmp(&self, other: &Self) -> core::option::Option<core::cmp::Ordering> {
                std::option::Option::Some(self.cmp(other))
            }
        }

        impl core::cmp::Ord for #struct_name {
            fn cmp(&self, other: &Self) -> core::cmp::Ordering {
                #ord_statement
            }
        }
    }
}

fn parse_fields(fields: FieldsNamed) -> Result<Vec<Expr>, Error> {
    let mut sortable_expressions = vec![];

    for field in fields.named {
        let span = field.span();
        let mut attrs = field
            .attrs
            .iter()
            .filter(|i| i.path.get_ident().map(|i| i == "sort_by") == Some(true));

        if attrs.next().is_none() {
            continue;
        }

        let expr: Expr = syn::parse2(field.ident.to_token_stream()).unwrap();
        sortable_expressions.push(expr);

        if attrs.next().is_some() {
            return Err(Error::new(
                span,
                r#"SortBy: expected at most one `sort_by` attribute"#,
            ));
        }
    }
    Ok(sortable_expressions)
}

fn parse_outer(attr: &Attribute) -> Result<Vec<Expr>, ()> {
    if let Ok(Meta::List(list)) = attr.parse_meta() {
        let mut sortable_fields = Vec::new();
        let mut valid = true;
        for name in list.nested {
            match name {
                NestedMeta::Meta(Meta::Path(p)) => {
                    let expr: Expr = syn::parse2(p.get_ident().to_token_stream()).unwrap();
                    sortable_fields.push(expr)
                }
                NestedMeta::Lit(Lit::Str(l)) => {
                    sortable_fields.push(l.parse().unwrap());
                }
                _ => {
                    valid = false;
                    break;
                }
            }
        }
        if valid {
            return Ok(sortable_fields);
        }
    }

    match syn::parse2::<Expr>(attr.tokens.clone()) {
        Ok(Expr::Tuple(tuple)) => {
            let elems = tuple.elems.into_iter().map(|elem| match elem {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(lit), ..
                }) => lit.parse().unwrap(),
                _ => elem,
            });
            return Ok(elems.collect());
        }
        Ok(Expr::Paren(expr)) => return Ok(vec![*expr.expr]),
        _ => (),
    }

    Err(())
}

#[cfg(test)]
mod test {
    use rust_format::Formatter;

    #[test]
    fn test_struct() {
        let input = syn::parse_quote! {
            #[sort_by("embed.otherfield")]
            struct Toto {
                #[sort_by]
                a: u16,
                #[sort_by]
                c: u32,
                b: f32,
                embed: EmbedStruct
            }
        };

        let output = crate::sort_by::impl_sort_by_derive(syn::parse2(input).unwrap());
        let output = rust_format::RustFmt::default()
            .format_str(output.to_string())
            .unwrap();
        assert_eq!(
            output,
            r#"impl std::hash::Hash for Toto {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.embed.otherfield.hash(state);
        self.a.hash(state);
        self.c.hash(state);
    }
}
impl core::cmp::Eq for Toto {}
impl core::cmp::PartialEq<Self> for Toto {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl core::cmp::PartialOrd<Self> for Toto {
    fn partial_cmp(&self, other: &Self) -> core::option::Option<core::cmp::Ordering> {
        std::option::Option::Some(self.cmp(other))
    }
}
impl core::cmp::Ord for Toto {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        core::cmp::Ord::cmp(&self.embed.otherfield, &other.embed.otherfield)
            .then_with(|| self.a.cmp(&other.a))
            .then_with(|| self.c.cmp(&other.c))
    }
}
"#
        );
    }

    #[test]
    fn test_enum() {
        let input = syn::parse_quote! {
            #[sort_by(get_something(), something.do_this())]
            #[accessor(global_time: usize)]
            enum Toto {
                A(u32),
                B,
                G { doesnotmatter: String, anyway: usize }
            }
        };

        let output = crate::sort_by::impl_sort_by_derive(syn::parse2(input).unwrap());
        let output = rust_format::RustFmt::default()
            .format_str(output.to_string())
            .unwrap();
        assert_eq!(
            output,
            r#"impl std::hash::Hash for Toto {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.get_something().hash(state);
        self.something.do_this().hash(state);
    }
}
impl core::cmp::Eq for Toto {}
impl core::cmp::PartialEq<Self> for Toto {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl core::cmp::PartialOrd<Self> for Toto {
    fn partial_cmp(&self, other: &Self) -> core::option::Option<core::cmp::Ordering> {
        std::option::Option::Some(self.cmp(other))
    }
}
impl core::cmp::Ord for Toto {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        core::cmp::Ord::cmp(&self.get_something(), &other.get_something())
            .then_with(|| self.something.do_this().cmp(&other.something.do_this()))
    }
}
"#
        );
    }

    #[test]
    fn test_singlecall() {
        let input = syn::parse_quote! {
            #[sort_by(get_something())]
            #[accessor(global_time: usize)]
            enum Toto {
                A(u32),
                B,
                G { doesnotmatter: String, anyway: usize }
            }
        };

        let output = crate::sort_by::impl_sort_by_derive(syn::parse2(input).unwrap());
        let output = rust_format::RustFmt::default()
            .format_str(output.to_string())
            .unwrap();
        assert_eq!(
            output,
            r#"impl std::hash::Hash for Toto {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.get_something().hash(state);
    }
}
impl core::cmp::Eq for Toto {}
impl core::cmp::PartialEq<Self> for Toto {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl core::cmp::PartialOrd<Self> for Toto {
    fn partial_cmp(&self, other: &Self) -> core::option::Option<core::cmp::Ordering> {
        std::option::Option::Some(self.cmp(other))
    }
}
impl core::cmp::Ord for Toto {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        core::cmp::Ord::cmp(&self.get_something(), &other.get_something())
    }
}
"#
        );
    }
}
