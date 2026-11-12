import FixedPoint.Spec
import FixedPoint.FloorCeilSpec
import FixedPoint.HalfUpSpec
import Audit

-- The rounding specifications shared by RatesAeneas, PoolAeneas and
-- ControllerAeneas: pure integer statements, no generated code.
-- Fails the build on any `sorry`, `native_decide` or added axiom (Audit.lean).
#assert_standard_axioms FixedPoint
