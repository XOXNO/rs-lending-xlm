#[macro_export]
macro_rules! apply_summary {
    (@mk_orig_module $id:ident, [$($prototype:tt)*], $( -> $ret:ty )?, $body:block) => {
        pub(crate) mod $id {
            use super::*;
            #[allow(dead_code)]
            #[allow(unused_variables)]
            pub(crate) fn $id($($prototype)*) $( -> $ret )? $body
        }
    };
    (@mk_orig $( #[$meta:meta] )*, $vis:vis, $id:ident, [$($prototype:tt)*], $( -> $ret:ty )?, $body:block) => {
        $( #[$meta] )*
        $vis fn $id($($prototype)*) $( -> $ret )? $body
    };
    ($new:path,
        $( #[$meta:meta]  )*
        $vis:vis fn $id:ident ($($arg:ident : $arg_ty:ty),* $(,)?) $( -> $ret:ty )?
        $body:block
    ) => {
        #[cfg(feature="certora")]
        $( #[$meta] )*
        pub(crate) fn $id($($arg : $arg_ty),*) $( -> $ret )? {
            $new($($arg),*)
        }

        #[cfg(feature="certora")]
        $crate::apply_summary!(
            @mk_orig_module $id, [$($arg : $arg_ty),*], $( -> $ret  )?, $body
        );

        #[cfg(not(feature="certora"))]
        $crate::apply_summary!(@mk_orig $( #[$meta] )*, $vis, $id, [$($arg : $arg_ty),*], $( -> $ret  )?, $body);
    };

    ($spec:ident, $old:ident,
        $( #[$meta:meta]  )*
        $vis:vis fn $id:ident (&mut $self:ident $( , )? $($arg:ident : $arg_ty:ty),* $(,)?) $( -> $ret:ty )?
        $body:block
    ) => {
        #[cfg(feature="certora")]
        $( #[$meta] )*
        pub(crate) fn $id(&mut $self, $($arg : $arg_ty),*) $( -> $ret )? {
            $self.$spec($($arg),*)
        }

        #[cfg(feature="certora")]
        pub(crate) fn $old(&mut $self, $($arg : $arg_ty),*) $( -> $ret )? $body

        #[cfg(not(feature="certora"))]
        $crate::apply_summary!(@mk_orig $( #[$meta] )*, $vis, $id, [&mut $self, $($arg : $arg_ty),*], $( -> $ret  )?, $body);
    };

    ($spec:ident, $old:ident,
        $( #[$meta:meta]  )*
        $vis:vis fn $id:ident (&$self:ident $( , )? $($arg:ident : $arg_ty:ty),* $(,)?) $( -> $ret:ty )?
        $body:block
    ) => {
        #[cfg(feature="certora")]
        $( #[$meta] )*
        pub(crate) fn $id(&$self, $($arg : $arg_ty),*) $( -> $ret )? {
            $self.$spec($($arg),*)
        }

        #[cfg(feature="certora")]
        pub(crate) fn $old(&$self, $($arg : $arg_ty),*) $( -> $ret )? $body

        #[cfg(not(feature="certora"))]
        $crate::apply_summary!(@mk_orig $( #[$meta] )*, $vis, $id, [&$self, $($arg : $arg_ty),*], $( -> $ret  )?, $body);
    };

    ($spec:ident, $old:ident,
        $( #[$meta:meta]  )*
        $vis:vis fn $id:ident ($self:ident $( , )? $($arg:ident : $arg_ty:ty),* $(,)?) $( -> $ret:ty )?
        $body:block
    ) => {
        #[cfg(feature="certora")]
        $( #[$meta] )*
        pub(crate) fn $id($self, $($arg : $arg_ty),*) $( -> $ret )? {
            $self.$spec($($arg),*)
        }

        #[cfg(feature="certora")]
        pub(crate) fn $old($self, $($arg : $arg_ty),*) $( -> $ret )? $body

        #[cfg(not(feature="certora"))]
        $crate::apply_summary!(@mk_orig $( #[$meta] )*, $vis, $id, [$self, $($arg : $arg_ty),*], $( -> $ret  )?, $body);
    };
}
