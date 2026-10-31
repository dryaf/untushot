// Return compiler diagnostics using syn; no panic-based macro error dependency.
macro_rules! abort_call_site {
    ($message:expr $(,)?) => {
        return syn::Error::new(proc_macro2::Span::call_site(), $message)
            .to_compile_error().into()
    };
    ($($message:tt)*) => {
        return syn::Error::new(proc_macro2::Span::call_site(), format!($($message)*))
            .to_compile_error().into()
    };
}
pub(crate) use abort_call_site;
