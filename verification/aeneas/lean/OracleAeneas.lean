import OracleAeneas.Funs
import OracleAeneas.Proofs.Host
import OracleAeneas.Proofs.Spec
import OracleAeneas.Proofs.Isqrt
import OracleAeneas.Proofs.HalfUp
import OracleAeneas.Proofs.Fp
import OracleAeneas.Proofs.Lp
import OracleAeneas.Proofs.LpStable
import OracleAeneas.Proofs.StableD
import Audit

-- Fails the build on any `sorry`, `native_decide` or added axiom in this
-- library: generated code, external models and proofs (Audit.lean).
#assert_standard_axioms OracleAeneas
