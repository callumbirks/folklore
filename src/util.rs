/// A macro to assert checks on generic type parameters at compile time.
/// Credits: <https://morestina.net/blog/1940/compile-time-checks-in-generic-functions-work-and-you-can-use-them-in-your-code>
#[macro_export]
macro_rules! generic_const_assert {
    ($t:ident, $check:expr) => {{
        #[allow(unused_attributes)]
        struct Check<$t>($t);
        impl<$t> Check<$t> {
            const CHECK: () = assert!($check);
        }
        let _ = Check::<$t>::CHECK;
    }};
}

#[macro_export]
macro_rules! generic_asserts {
    (($($l:lifetime,)* $($($t:ident$(: $bound:path)?),+)? $(; $(const $c:ident:$ct:ty),+)?);
    $($label:ident: $test:expr);+$(;)?) => {
    #[allow(path_statements, clippy::no_effect)]
    {
        struct Check<$($l,)* $($($t,)+)? $($(const $c:$ct,)+)?>($($($t,)+)?);
        impl<$($l,)* $($($t$(:$bound)?,)+)? $($(const $c:$ct,)+)?> Check<$($l,)* $($($t,)+)? $($($c,)+)?> {
            $( const $label: () = assert!($test); )+
        }
        generic_asserts!{@nested Check::<$($l,)* $($($t,)+)? $($($c,)+)?>, $($label: $test;)+}
    }};
    (@nested $t:ty, $($label:ident: $test:expr;)+) => {
        $(<$t>::$label;)+
    }
}
