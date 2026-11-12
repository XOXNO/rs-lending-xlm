//! The production liquidation curve, liquidation math and liquidation
//! application, verbatim; `bad_debt` is a stand-in.

#[path = "../../../../../../contracts/controller/src/positions/liquidation/apply.rs"]
pub mod apply;
pub mod bad_debt;
#[path = "../../../../../../contracts/controller/src/positions/liquidation/curve.rs"]
pub mod curve;
#[path = "../../../../../../contracts/controller/src/positions/liquidation/math.rs"]
pub mod math;
