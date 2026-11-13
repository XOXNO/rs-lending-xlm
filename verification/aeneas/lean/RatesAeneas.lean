import RatesAeneas.Funs
import FixedPoint.Spec
import RatesAeneas.Proofs.Host
import RatesAeneas.Proofs.MulDiv
import RatesAeneas.Proofs.Scaling
-- Floor and ceiling rounding, share conversions (docs/rates-floor-ceil.md)
import FixedPoint.FloorCeilSpec
import RatesAeneas.Proofs.FloorCeilCore
import RatesAeneas.Proofs.FloorCeilRescale
import RatesAeneas.Proofs.FloorCeilScaling
import RatesAeneas.Proofs.FloorCeilRoundTrip
import RatesAeneas.Proofs.FloorCeilResolve
-- Half-up rounding, rate curve, index accrual (docs/rates-half-up-curve-index.md)
import FixedPoint.HalfUpSpec
import RatesAeneas.Proofs.HalfUp
import RatesAeneas.Proofs.HalfUpDivInt
import RatesAeneas.Proofs.HalfUpWrappers
import RatesAeneas.Proofs.CurveBase
import RatesAeneas.Proofs.Curve
import RatesAeneas.Proofs.CurveRates
import RatesAeneas.Proofs.Index
-- Typed fixed-point wrappers added by #224 (both docs)
import RatesAeneas.Proofs.TypedWrappers
-- resolve_withdrawal (docs/rates-floor-ceil.md)
import RatesAeneas.Proofs.Withdrawal
import Audit

-- Fails the build on any `sorry`, `native_decide` or added axiom in this
-- library: generated code, external models and proofs (Audit.lean).
#assert_standard_axioms RatesAeneas
