// Function summaries for Certora verification.
//
// Complex functions are abstracted to make verification feasible.
// Each summary must soundly over-approximate the real function's behavior
// with respect to the properties being verified.
//
// Summaries model only the state changes relevant to the invariant
// being checked, not full behavior.

// Summaries will be added as needed when running the prover.
// The prover will indicate which functions are too complex and need
// summarization. Common candidates:
//
// - Oracle price fetching (complex external calls)
// - Interest accrual (Taylor expansion arithmetic)
// - Cross-contract pool calls (external invocations)
// - Token transfers (external contract calls)
