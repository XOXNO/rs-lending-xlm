use cvlr_macros::cvlr_assert_that;
pub fn test_guarded_comparisons() {
    let flag = true;
    let cond = false;
    let guard = true;
    let test = true;
    let check = false;
    let a = 1;
    let b = 2;
    let c = 3;
    let d = 4;
    let p = 5;
    let q = 6;
    let x = 6;
    let y = 7;
    let z = 8;
    let m = 9;
    let n = 10;
    if flag {
        {
            let c_ = a < b;
            ::cvlr_asserts::log::add_loc("<FILE>", 0u32);
            ::cvlr_asserts::cvlr_assert_checked(c_);
        };
    } else {
        ()
    };
    if x > 0 {
        {
            let c_ = y <= z;
            ::cvlr_asserts::log::add_loc("<FILE>", 0u32);
            ::cvlr_asserts::cvlr_assert_checked(c_);
        };
    } else {
        ()
    };
    if cond {
        {
            let c_ = p > q;
            ::cvlr_asserts::log::add_loc("<FILE>", 0u32);
            ::cvlr_asserts::cvlr_assert_checked(c_);
        };
    } else {
        ()
    };
    if guard {
        {
            let c_ = m >= n;
            ::cvlr_asserts::log::add_loc("<FILE>", 0u32);
            ::cvlr_asserts::cvlr_assert_checked(c_);
        };
    } else {
        ()
    };
    if test {
        {
            let c_ = x == y;
            ::cvlr_asserts::log::add_loc("<FILE>", 0u32);
            ::cvlr_asserts::cvlr_assert_checked(c_);
        };
    } else {
        ()
    };
    if check {
        {
            let c_ = a != b;
            ::cvlr_asserts::log::add_loc("<FILE>", 0u32);
            ::cvlr_asserts::cvlr_assert_checked(c_);
        };
    } else {
        ()
    };
    if a > c {
        {
            let c_ = d < p;
            ::cvlr_asserts::log::add_loc("<FILE>", 0u32);
            ::cvlr_asserts::cvlr_assert_checked(c_);
        };
    } else {
        ()
    };
    if x + 1 > 0 {
        {
            let c_ = y * 2 < z;
            ::cvlr_asserts::log::add_loc("<FILE>", 0u32);
            ::cvlr_asserts::cvlr_assert_checked(c_);
        };
    } else {
        ()
    };
}
pub fn main() {}
