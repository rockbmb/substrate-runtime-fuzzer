//! Writes seed inputs for the kitchensink fuzzer.
//!
//! The harness decodes a stream of `(advance_block: bool, origin: u8,
//! RuntimeCall)` tuples, prefixed by one byte that selects the price replay's
//! starting tick. Seeds encode call sequences the mutator does not find on
//! its own; the first one is the mint-then-redeem pair on the 2-decimal
//! external, whose truncation path no coverage signal distinguishes from the
//! zero-debt error path, so nothing guides the mutator toward the ordered
//! sequence.

use codec::Encode;
use kitchensink_runtime::RuntimeCall;
use sp_runtime::Permill;

fn main() {
    let out = std::env::args().nth(1).expect("usage: seedgen <out-dir>");

    let mint = RuntimeCall::Psm(pallet_psm::Call::mint {
        internal_asset: 1,
        external_asset: 3,
        external_amount: 10_000,
        max_fee: Permill::one(),
    });
    let redeem = RuntimeCall::Psm(pallet_psm::Call::redeem {
        internal_asset: 1,
        external_asset: 3,
        internal_amount: 1_234_567,
        max_fee: Permill::one(),
    });

    let mut data = vec![0u8];
    (false, 0u8, mint).encode_to(&mut data);
    (false, 0u8, redeem).encode_to(&mut data);

    let path = format!("{out}/seed-psm-mint-redeem-usdx");
    std::fs::write(&path, &data).expect("write seed");
    println!("{path}: {} bytes", data.len());
}
