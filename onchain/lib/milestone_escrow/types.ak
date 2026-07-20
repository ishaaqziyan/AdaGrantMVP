use aiken/crypto.{VerificationKeyHash}
use cardano/assets.{PolicyId}

/// State of a milestone-based grant escrow, held inline at the script UTxO.
///
/// `tranche_bps`, `approved` are parallel lists, one entry per milestone
/// (index 0..length-1). `tranche_bps` must sum to 10000 (basis points) and
/// is fixed for the lifetime of the escrow; `total_locked` is the ADA
/// amount originally deposited, used as the fixed base for every tranche
/// percentage so payouts don't drift as the script balance shrinks.
///
/// `receipt_policy_id` lives in the datum rather than as a validator
/// parameter: `milestone_escrow`'s own script hash is a parameter of
/// `milestone_receipt`, so making the receipt policy ID a parameter of
/// `milestone_escrow` in turn would make the two scripts' hashes depend on
/// each other -- unresolvable. Setting it at escrow-creation time (after
/// `milestone_receipt` has already been compiled against the escrow's
/// hash) breaks the cycle.
pub type Datum {
  reviewer: VerificationKeyHash,
  proposer: VerificationKeyHash,
  total_locked: Int,
  tranche_bps: List<Int>,
  approved: List<Bool>,
  released_count: Int,
  receipt_policy_id: PolicyId,
}

pub type Redeemer {
  ApproveMilestone(Int)
  ReleaseTranche(Int)
}
