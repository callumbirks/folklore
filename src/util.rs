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

#[macro_export]
macro_rules! wrap {
    (<$to_ty:ty>: $in:expr,$cap:expr) => {
        ($in as usize % $cap as usize) as $to_ty
    };
}

pub const fn get_bucket_index(index: usize, size_mask: usize) -> usize {
    (index & size_mask) >> 2
}

#[allow(clippy::cast_possible_truncation)]
pub const fn get_cell_index(index: usize) -> u8 {
    (index & 3) as u8
}

use core::hash::Hash;
use hash32::Hasher;

pub fn hash<T: ?Sized, H>(key: &T) -> u32
where
    T: Hash,
    H: Hasher + Default,
{
    let mut hasher = H::default();
    key.hash(&mut hasher);
    hasher.finish32()
}

pub fn allocate<T>(count: usize) -> *mut T {
    let layout = core::alloc::Layout::array::<T>(count).unwrap();
    unsafe { alloc::alloc::alloc(layout).cast::<T>() }
}

pub fn allocate_zeroed<T>(count: usize) -> *mut T {
    let layout = core::alloc::Layout::array::<T>(count).unwrap();
    unsafe { alloc::alloc::alloc_zeroed(layout).cast::<T>() }
}

pub fn deallocate<T>(ptr: *mut T, count: usize) {
    let layout = core::alloc::Layout::array::<T>(count).unwrap();
    unsafe { alloc::alloc::dealloc(ptr.cast::<u8>(), layout) }
}
