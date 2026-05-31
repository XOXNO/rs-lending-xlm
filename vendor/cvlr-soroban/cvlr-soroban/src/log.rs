pub struct CvlrAddress<'a>(pub &'a soroban_sdk::Address);
pub struct CvlrSymbol<'a>(pub &'a soroban_sdk::Symbol);

pub struct CvlrBytesN<'a>(pub &'a soroban_sdk::BytesN<32>);
pub struct CvlrBytes<'a>(pub &'a soroban_sdk::Bytes);

pub trait SorobanAsCvlr<'a> {
    type Cvlr;
    fn as_cvlr(&'a self) -> Self::Cvlr;
}

macro_rules! impl_cvlr_wrapper {
    ($wrapper:ident, $soroban:ty) => {
        impl<'a> SorobanAsCvlr<'a> for $soroban {
            type Cvlr = $wrapper<'a>;
            #[inline(always)]
            fn as_cvlr(&'a self) -> $wrapper<'a> {
                $wrapper(self)
            }
        }

        impl<'a> From<$wrapper<'a>> for &'a $soroban {
            #[inline(always)]
            fn from(v: $wrapper<'a>) -> Self {
                v.0
            }
        }

        impl cvlr_log::CvlrLog for $wrapper<'_> {
            #[inline(always)]
            fn log(&self, tag: &str, logger: &mut cvlr_log::CvlrLogger) {
                logger.log_u64(tag, self.0.to_val().get_payload() >> 8);
            }
        }
    };
}

// lower 8 bits is tag (77), we want the upper 56 bits which is a u64
impl_cvlr_wrapper!(CvlrAddress, soroban_sdk::Address);

// lower 8 bits is tag (74)
// Note: the log only works if Symbol was created with nondet_symbol().
impl_cvlr_wrapper!(CvlrSymbol, soroban_sdk::Symbol);

// Note: the log only works if Bytes was created with nondet_bytes_n().
impl_cvlr_wrapper!(CvlrBytesN, soroban_sdk::BytesN<32>);

// Note: the log only works if Bytes was created with nondet_bytes1().
impl_cvlr_wrapper!(CvlrBytes, soroban_sdk::Bytes);
