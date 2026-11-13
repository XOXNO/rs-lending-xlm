-- Pool accounting proofs (see ../docs/pool.md).
import PoolAeneas.Funs
-- Math proofs copied from RatesAeneas (namespace renamed).
import FixedPoint.Spec
import PoolAeneas.Proofs.Host
import PoolAeneas.Proofs.MulDiv
import PoolAeneas.Proofs.Scaling
import FixedPoint.FloorCeilSpec
import PoolAeneas.Proofs.FloorCeilCore
import PoolAeneas.Proofs.FloorCeilRescale
import PoolAeneas.Proofs.FloorCeilScaling
import FixedPoint.HalfUpSpec
import PoolAeneas.Proofs.HalfUp
import PoolAeneas.Proofs.HalfUpDivInt
import PoolAeneas.Proofs.FloorCeilRoundTrip
import PoolAeneas.Proofs.FloorCeilResolve
import PoolAeneas.Proofs.Withdrawal
import PoolAeneas.Proofs.RatesHalfUpWrappers
import PoolAeneas.Proofs.RatesCurveBase
import PoolAeneas.Proofs.RatesCurve
import PoolAeneas.Proofs.RatesCurveRates
import PoolAeneas.Proofs.RatesIndex
-- Pool proofs.
import PoolAeneas.Proofs.Basic
import PoolAeneas.Proofs.Wrappers
import PoolAeneas.Proofs.Shares
import PoolAeneas.Proofs.Guards
import PoolAeneas.Proofs.Cash
import PoolAeneas.Proofs.Interest
import PoolAeneas.Proofs.Flash
import PoolAeneas.Proofs.Compound
import PoolAeneas.Proofs.Accrual
import PoolAeneas.Proofs.GlobalSync
-- Operation legs and the Certora pool summaries.
import PoolAeneas.Proofs.Legs
import PoolAeneas.Proofs.Legs2
import PoolAeneas.Proofs.Summaries
import Audit

-- Fails the build on any `sorry`, `native_decide` or added axiom in this
-- library: generated code, external models and proofs (Audit.lean).
#assert_standard_axioms PoolAeneas
