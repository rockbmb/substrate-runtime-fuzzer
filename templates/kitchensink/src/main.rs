#![warn(clippy::pedantic)]
use codec::{DecodeLimit, Encode};
use frame_support::{
    dispatch::GetDispatchInfo,
    pallet_prelude::Weight,
    traits::{IntegrityTest, OriginTrait, TryState, TryStateSelect},
    weights::constants::WEIGHT_REF_TIME_PER_SECOND,
};
use frame_system::Account;
use kitchensink_runtime::{
    constants::{currency::DOLLARS, time::SLOT_DURATION},
    AccountId, AllPalletsWithSystem, Balances, Broker, Executive, Runtime, RuntimeCall,
    RuntimeOrigin, Timestamp,
};
use node_primitives::Balance;
use pallet_balances::{Holds, TotalIssuance};
use sp_consensus_babe::{
    digests::{PreDigest, SecondaryPlainPreDigest},
    Slot, BABE_ENGINE_ID,
};
use sp_runtime::{
    testing::H256,
    traits::{Dispatchable, Header},
    Digest, DigestItem, FixedU64, Perbill, Storage,
};
use sp_state_machine::BasicExternalities;
use std::{
    iter,
    time::{Duration, Instant},
};

mod oct10;

fn main() {
    let accounts: Vec<AccountId> = (0..5).map(|i| [i; 32].into()).collect();
    let genesis = generate_genesis(&accounts);

    ziggy::fuzz!(|data: &[u8]| {
        process_input(&accounts, &genesis, data);
    });
}
#[allow(clippy::too_many_lines)]
fn generate_genesis(accounts: &[AccountId]) -> Storage {
    use kitchensink_runtime::{
        AllianceConfig, AllianceMotionConfig, AssetConversionConfig, AssetsConfig,
        AuthorityDiscoveryConfig, BabeConfig, BalancesConfig, BeefyConfig, BrokerConfig,
        CouncilConfig, DemocracyConfig, ElectionsConfig, GluttonConfig, GrandpaConfig,
        ImOnlineConfig, IndicesConfig, MixnetConfig, NominationPoolsConfig, PoolAssetsConfig,
        ReviveConfig, RuntimeGenesisConfig, SafeModeConfig, SessionConfig, SessionKeys,
        SocietyConfig, StakingConfig, SudoConfig, SystemConfig, TechnicalCommitteeConfig,
        TechnicalMembershipConfig, TransactionPaymentConfig, TransactionStorageConfig,
        TreasuryConfig, TxPauseConfig, VestingConfig,
    };
    use pallet_grandpa::AuthorityId as GrandpaId;
    use pallet_im_online::sr25519::AuthorityId as ImOnlineId;
    use pallet_staking::StakerStatus;
    use sp_authority_discovery::AuthorityId as AuthorityDiscoveryId;
    use sp_consensus_babe::AuthorityId as BabeId;
    use sp_core::{sr25519::Public as MixnetId, Pair};
    use sp_runtime::{app_crypto::ByteArray, BuildStorage};

    const ENDOWMENT: Balance = 10_000_000 * DOLLARS;
    const STASH: Balance = ENDOWMENT / 1000;

    let beefy_pair = sp_consensus_beefy::ecdsa_crypto::Pair::generate().0;

    let stakers = vec![(
        [0; 32].into(),
        [0; 32].into(),
        STASH,
        StakerStatus::Validator,
    )];

    let num_endowed_accounts = accounts.len();

    let mut storage = RuntimeGenesisConfig {
        system: SystemConfig::default(),
        balances: BalancesConfig {
            balances: accounts.iter().cloned().map(|x| (x, ENDOWMENT)).collect(),
            dev_accounts: None,
        },
        indices: IndicesConfig { indices: vec![] },
        session: SessionConfig {
            keys: vec![(
                [0; 32].into(),
                [0; 32].into(),
                SessionKeys {
                    grandpa: GrandpaId::from_slice(&[0; 32]).unwrap(),
                    babe: BabeId::from_slice(&[0; 32]).unwrap(),
                    beefy: beefy_pair.public(),
                    im_online: ImOnlineId::from_slice(&[0; 32]).unwrap(),
                    authority_discovery: AuthorityDiscoveryId::from_slice(&[0; 32]).unwrap(),
                    mixnet: MixnetId::from_slice(&[0; 32]).unwrap().into(),
                },
            )],
            non_authority_keys: vec![],
        },
        beefy: BeefyConfig::default(),
        staking: StakingConfig {
            validator_count: 0u32,
            minimum_validator_count: 0u32,
            invulnerables: vec![[0; 32].into()],
            slash_reward_fraction: Perbill::from_percent(10),
            stakers,
            ..Default::default()
        },
        democracy: DemocracyConfig::default(),
        elections: ElectionsConfig {
            members: accounts
                .iter()
                .take(num_endowed_accounts.div_ceil(2))
                .cloned()
                .map(|member| (member, STASH))
                .collect(),
        },
        council: CouncilConfig::default(),
        technical_committee: TechnicalCommitteeConfig {
            members: accounts
                .iter()
                .take(num_endowed_accounts.div_ceil(2))
                .cloned()
                .collect(),
            ..Default::default()
        },
        sudo: SudoConfig { key: None },
        babe: BabeConfig {
            authorities: vec![],
            epoch_config: kitchensink_runtime::BABE_GENESIS_EPOCH_CONFIG,
            ..Default::default()
        },
        im_online: ImOnlineConfig { keys: vec![] },
        authority_discovery: AuthorityDiscoveryConfig::default(),
        grandpa: GrandpaConfig::default(),
        technical_membership: TechnicalMembershipConfig::default(),
        treasury: TreasuryConfig::default(),
        society: SocietyConfig { pot: 0 },
        vesting: VestingConfig::default(),
        assets: AssetsConfig {
            // This asset is used by the NIS pallet as counterpart currency.
            assets: vec![(9, [0; 32].into(), true, 1)],
            ..Default::default()
        },
        transaction_storage: TransactionStorageConfig::default(),
        transaction_payment: TransactionPaymentConfig::default(),
        alliance: AllianceConfig::default(),
        alliance_motion: AllianceMotionConfig::default(),
        nomination_pools: NominationPoolsConfig {
            min_create_bond: 10 * DOLLARS,
            min_join_bond: DOLLARS,
            ..Default::default()
        },
        glutton: GluttonConfig {
            compute: FixedU64::default(),
            storage: FixedU64::default(),
            trash_data_count: Default::default(),
            ..Default::default()
        },
        pool_assets: PoolAssetsConfig::default(),
        safe_mode: SafeModeConfig::default(),
        tx_pause: TxPauseConfig::default(),
        mixnet: MixnetConfig::default(),
        broker: BrokerConfig::default(),
        revive: ReviveConfig::default(),
        asset_conversion: AssetConversionConfig::default(),
    }
    .build_storage()
    .unwrap();
    BasicExternalities::execute_with_storage(&mut storage, || {
        // We set the configuration for the broker pallet
        Broker::configure(
            RuntimeOrigin::root(),
            pallet_broker::ConfigRecord {
                advance_notice: 2,
                interlude_length: 1,
                leadin_length: 1,
                ideal_bulk_proportion: Perbill::default(),
                limit_cores_offered: None,
                region_length: 3,
                renewal_bump: Perbill::from_percent(10),
                contribution_timeout: 5,
            },
        )
        .unwrap();

        // pUSD system setup. Without a registered PSM the pallet's calls all
        // return PsmNotFound, so the fuzzer would never reach its logic.
        //
        // Asset 1 is the stablecoin the PSM mints; asset 2 is the external it
        // holds in reserve. Both carry six decimals, which the PSM records at
        // registration.
        let owner = AccountId::from([0; 32]);
        for (id, symbol) in [(1u32, b"PUSD".to_vec()), (2u32, b"USDT".to_vec())] {
            kitchensink_runtime::Assets::force_create(
                RuntimeOrigin::root(),
                id.into(),
                owner.clone().into(),
                true,
                1,
            )
            .unwrap();
            kitchensink_runtime::Assets::set_metadata(
                RuntimeOrigin::signed(owner.clone()),
                id.into(),
                symbol.clone(),
                symbol,
                6,
            )
            .unwrap();
        }

        // Fund the fuzzer's accounts with the external asset so mints can happen.
        for i in 0..5u8 {
            kitchensink_runtime::Assets::mint(
                RuntimeOrigin::signed(owner.clone()),
                2u32.into(),
                AccountId::from([i; 32]).into(),
                1_000_000_000_000,
            )
            .unwrap();
        }

        let admin = kitchensink_runtime::OriginCaller::system(frame_system::RawOrigin::Signed(owner.clone()));
        kitchensink_runtime::Psm::create_psm(
            RuntimeOrigin::signed(owner.clone()),
            1u32,
            Box::new(admin.clone()),
            Box::new(admin),
            owner.clone(),
            1_000_000_000_000,
            1_000_000,
        )
        .unwrap();
        kitchensink_runtime::Psm::add_external_asset(RuntimeOrigin::signed(owner.clone()), 1u32, 2u32)
            .unwrap();
        kitchensink_runtime::Psm::set_asset_ceiling_weight(
            RuntimeOrigin::signed(owner.clone()),
            1u32,
            2u32,
            sp_runtime::Permill::one(),
        )
        .unwrap();

        // A vault market over the same stablecoin, so the two pallets mint the
        // same asset and the cross-pallet checks have both halves to compare.
        // Without a branch, every check in pallet-vaults iterates empty maps.
        //
        // `do_create_branch` reads the oracle before anything else, so the price
        // has to exist first. The value is a FixedU128 inner, scaled by 10^18;
        // the key for native collateral is `VaultsNativePriceFeedId`, u32::MAX.
        // The oracle combines a price only once `MinimumCount` distinct members
        // have fed it, so the feeders must be members first. Membership is set
        // here rather than in genesis, where it would re-initialize the
        // technical committee that its own genesis already built.
        for i in 0..5u8 {
            kitchensink_runtime::TechnicalMembership::add_member(
                RuntimeOrigin::root(),
                AccountId::from([i; 32]).into(),
            )
            .unwrap();
        }

        for i in 0..5u8 {
            kitchensink_runtime::Oracle::feed_values(
                RuntimeOrigin::signed(AccountId::from([i; 32])),
                vec![(u32::MAX, price_replay::price_at(0, 0))]
                    .try_into()
                    .unwrap(),
            )
            .unwrap();
        }

        kitchensink_runtime::Vaults::create_branch(
            RuntimeOrigin::root(),
            frame_support::traits::fungible::NativeOrWithId::Native,
            1u32,
            pallet_vaults::types::BranchAdmins {
                full_admin: owner.clone().into(),
                emergency_admin: owner.into(),
            },
            pallet_vaults::types::BranchConfig {
                minimum_collateralization_ratio: sp_runtime::FixedU128::from_rational(110, 100),
                initial_collateralization_ratio: sp_runtime::FixedU128::from_rational(120, 100),
                safety_collateralization_ratio: sp_runtime::FixedU128::from_rational(130, 100),
                debt_ceiling: 1_000_000_000_000,
                minimum_debt: 200,
                minimum_collateral: 10 * DOLLARS,
                minimum_borrow_rate: sp_runtime::FixedU128::from_rational(1, 1000),
                maximum_borrow_rate: sp_runtime::FixedU128::from_rational(400, 100),
                upfront_fee_period: 604_800_000,
                rate_adjustment_cooldown: 86_400_000,
                redistribution_penalty: sp_runtime::Permill::from_percent(5),
            },
            (),
        )
        .unwrap();

        // The fuzzer dispatches from these five accounts. While one of them owns
        // the stablecoin it can call `Assets::mint` and raise issuance without
        // either pallet recording debt, which is not a defect the cross-pallet
        // check is meant to report. Ownership moves to an account the fuzzer
        // never signs as. Both pallets mint through `fungibles`, not ownership,
        // so they are unaffected.
        kitchensink_runtime::Assets::set_team(
            RuntimeOrigin::signed(AccountId::from([0u8; 32])),
            1u32.into(),
            AccountId::from([9u8; 32]).into(),
            AccountId::from([9u8; 32]).into(),
            AccountId::from([9u8; 32]).into(),
        )
        .unwrap();
        // Vaults opened at the ICR floor, so the first ticks of the replayed
        // crash push them under MCR and the liquidation paths become one call
        // away for the fuzzer. The boundary debt is probed rather than derived:
        // binary-search the largest accepted debt at fixed collateral, then
        // open at 95% of it. PUSD_GENESIS_VAULTS picks the count; later
        // presets can vary the distribution.
        kitchensink_runtime::Vaults::set_global_debt_ceiling(
            RuntimeOrigin::root(),
            1u32,
            1_000_000_000_000,
        )
        .unwrap();

        // Genesis vault population, from PUSD_GENESIS_VAULTS: "count@CR"
        // bands, comma-separated. "20@2.5" is a healthy herd the crash should
        // not kill; "5@1.21" hugs the ICR floor and goes underwater within the
        // first slide; "10@2.5,10@1.21" splits the two. Default three vaults
        // just above the floor.
        //
        // Debt for a target CR comes from the probed ICR boundary rather than
        // decimal arithmetic: one binary search finds the largest debt the
        // pallet accepts at this collateral (CR there equals ICR by
        // definition), and debt = boundary * ICR / CR gives any other ratio.
        // The probe needs a pUSD buffer to repay its trial vaults, because
        // open charges an upfront fee; a PSM mint provides it and records
        // matching debt, so the cross-pallet equality is unaffected.
        //
        // Owners are accounts [100+j; 32], outside the fuzzer's five origins,
        // so fuzzed calls manage these vaults only through permissionless
        // paths (liquidate, poke, repay_for), the same way a stranger would.
        const GENESIS_ICR: f64 = 1.2;
        let bands: Vec<(u32, f64)> = std::env::var("PUSD_GENESIS_VAULTS")
            .unwrap_or_else(|_| "3@1.26".into())
            .split(',')
            .map(|band| {
                let (n, cr) = band.split_once('@').expect("band is count@CR");
                let n: u32 = n.trim().parse().expect("count parses");
                let cr: f64 = cr.trim().parse().expect("CR parses");
                assert!(cr >= 1.21, "CR below the ICR floor cannot be opened");
                (n, cr)
            })
            .collect();
        // Small enough that the ICR bound, not the branch debt ceiling, is
        // what the probe finds: the ceiling is shared across the branch, so a
        // ceiling-bound vault fills it alone and leaves the band's CR
        // arithmetic meaningless. 10 DOLLARS keeps thirty vaults under 30% of
        // the ceiling.
        let collateral: Balance = 10 * DOLLARS;

        let probe_owner = AccountId::from([99u8; 32]);
        kitchensink_runtime::Balances::transfer_allow_death(
            RuntimeOrigin::signed(AccountId::from([0u8; 32])),
            probe_owner.clone().into(),
            100_000 * DOLLARS,
        )
        .unwrap();
        kitchensink_runtime::Assets::mint(
            RuntimeOrigin::signed(AccountId::from([0u8; 32])),
            2u32.into(),
            probe_owner.clone().into(),
            1_000_000_000_000,
        )
        .unwrap();
        kitchensink_runtime::Psm::mint(
            RuntimeOrigin::signed(probe_owner.clone()),
            1u32,
            2u32,
            100_000_000_000,
            sp_runtime::Permill::one(),
        )
        .unwrap();
        // Each vault gets its own rate, rising with the owner index. Same-rate
        // inserts pile into one cluster that the endpoints-only hint cannot
        // reach once the cluster outgrows the linked list's repair budget;
        // distinct rising rates land every insert at the head end, which the
        // hint reaches directly. Distinct rates also spread the redemption
        // order, which same-rate genesis would collapse.
        let open_as = |who: &AccountId, rate_bps: u32, debt: Balance| {
            kitchensink_runtime::Vaults::open_vault(
                RuntimeOrigin::signed(who.clone()),
                frame_support::traits::fungible::NativeOrWithId::Native,
                1u32,
                collateral,
                debt,
                sp_runtime::FixedU128::from_rational(rate_bps as u128, 10_000),
                linked_list_interface::Position::endpoints_only(),
            )
        };
        let (mut lo, mut hi): (Balance, Balance) = (1, 1_000_000_000_000_000);
        while lo + 1 < hi {
            let mid = lo + (hi - lo) / 2;
            if open_as(&probe_owner, 500, mid).is_ok() {
                kitchensink_runtime::Vaults::repay_for(
                    RuntimeOrigin::signed(probe_owner.clone()),
                    frame_support::traits::fungible::NativeOrWithId::Native,
                    1u32,
                    probe_owner.clone().into(),
                    None,
                )
                .expect("probe repay");
                kitchensink_runtime::Vaults::close_vault(
                    RuntimeOrigin::signed(probe_owner.clone()),
                    frame_support::traits::fungible::NativeOrWithId::Native,
                    1u32,
                    None,
                )
                .expect("probe close");
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let boundary = lo;
        if std::env::var("PUSD_PROBE").is_ok() {
            eprintln!("genesis: probed ICR-boundary debt = {boundary}");
        }

        let mut owner_index: u8 = 100;
        for (count, cr) in bands {
            let debt = (boundary as f64 * GENESIS_ICR / cr) as Balance;
            if std::env::var("PUSD_PROBE").is_ok() {
                eprintln!("genesis: band {count}@{cr} -> debt {debt}");
            }
            for _ in 0..count {
                let who = AccountId::from([owner_index; 32]);
                owner_index = owner_index.checked_add(1).expect("fewer than 156 vaults");
                kitchensink_runtime::Balances::transfer_allow_death(
                    RuntimeOrigin::signed(AccountId::from([0u8; 32])),
                    who.clone().into(),
                    2_000 * DOLLARS,
                )
                .unwrap();
                open_as(&who, 500 + owner_index as u32, debt).expect("open at the band's CR");
            }
        }

        kitchensink_runtime::Balances::transfer_allow_death(
            RuntimeOrigin::signed(AccountId::from([0u8; 32])),
            AccountId::from([9u8; 32]).into(),
            1_000 * DOLLARS,
        )
        .unwrap();
        kitchensink_runtime::Assets::transfer_ownership(
            RuntimeOrigin::signed(AccountId::from([0u8; 32])),
            1u32.into(),
            AccountId::from([9u8; 32]).into(),
        )
        .unwrap();
    });
    storage
}

fn recursively_find_call(call: RuntimeCall, matches_on: fn(&RuntimeCall) -> bool) -> bool {
    if let RuntimeCall::Utility(
        pallet_utility::Call::batch { calls }
        | pallet_utility::Call::force_batch { calls }
        | pallet_utility::Call::batch_all { calls },
    ) = call
    {
        for call in calls {
            if recursively_find_call(call.clone(), matches_on) {
                return true;
            }
        }
    } else if let RuntimeCall::Utility(pallet_utility::Call::if_else { main, fallback }) = call {
        return recursively_find_call(*main.clone(), matches_on)
            || recursively_find_call(*fallback.clone(), matches_on);
    } else if let RuntimeCall::Lottery(pallet_lottery::Call::buy_ticket { call })
    | RuntimeCall::Multisig(pallet_multisig::Call::as_multi_threshold_1 {
        call, ..
    })
    | RuntimeCall::Utility(
        pallet_utility::Call::as_derivative { call, .. }
        | pallet_utility::Call::with_weight { call, .. }
        | pallet_utility::Call::dispatch_as_fallible { call, .. }
        | pallet_utility::Call::dispatch_as { call, .. },
    )
    | RuntimeCall::Sudo(
        pallet_sudo::Call::sudo { call, .. }
        | pallet_sudo::Call::sudo_unchecked_weight { call, .. },
    )
    | RuntimeCall::Whitelist(
        pallet_whitelist::Call::dispatch_whitelisted_call_with_preimage { call, .. },
    )
    | RuntimeCall::Proxy(
        pallet_proxy::Call::proxy { call, .. } | pallet_proxy::Call::proxy_announced { call, .. },
    )
    | RuntimeCall::Revive(
        pallet_revive::Call::dispatch_as_fallback_account { call }
        | pallet_revive::Call::eth_substrate_call { call, .. },
    )
    | RuntimeCall::Recovery(pallet_recovery::Call::control_inherited_account {
        call,
        ..
    })
    | RuntimeCall::Council(
        pallet_collective::Call::propose { proposal: call, .. }
        | pallet_collective::Call::execute { proposal: call, .. },
    )
    | RuntimeCall::AllianceMotion(
        pallet_collective::Call::propose { proposal: call, .. }
        | pallet_collective::Call::execute { proposal: call, .. },
    )
    | RuntimeCall::TechnicalCommittee(
        pallet_collective::Call::propose { proposal: call, .. }
        | pallet_collective::Call::execute { proposal: call, .. },
    ) = call
    {
        return recursively_find_call(*call, matches_on);
    } else if matches_on(&call) {
        return true;
    }
    false
}

fn call_filter(call: &RuntimeCall) -> bool {
    // We disallow referenda calls with root origin
    matches!(
        &call,
        RuntimeCall::Referenda(pallet_referenda::Call::submit {
            proposal_origin: matching_origin,
            ..
        }) | RuntimeCall::RankedPolls(pallet_referenda::Call::submit {
            proposal_origin: matching_origin,
            ..
        }) if RuntimeOrigin::from(*matching_origin.clone()).caller() == RuntimeOrigin::root().caller()
    )
    // We disallow batches of referenda
    || matches!(
            &call,
            RuntimeCall::Referenda(pallet_referenda::Call::submit { .. })
        )
    // We filter out contracts call that will take too long because of fuzzer instrumentation
    || matches!(
            &call,
            RuntimeCall::Contracts(
                pallet_contracts::Call::instantiate_with_code { .. } |
                pallet_contracts::Call::upload_code { .. } |
                pallet_contracts::Call::instantiate_with_code_old_weight { .. } |
                pallet_contracts::Call::migrate { .. }
            )
        )
    || matches!(
            &call,
            RuntimeCall::Revive(
                pallet_revive::Call::instantiate_with_code { .. } |
                pallet_revive::Call::upload_code { .. }
            )
        )
    // We filter out safe_mode calls, as they block timestamps from being set.
    || matches!(&call, RuntimeCall::SafeMode(..))
    // We filter out store extrinsics because BasicExternalities does not support them.
    || matches!(
            &call,
            RuntimeCall::TransactionStorage(pallet_transaction_storage::Call::store { .. })
                | RuntimeCall::Remark(pallet_remark::Call::store { .. })
        )
    || matches!(
            &call,
            RuntimeCall::NominationPools(..)
    )
    || matches!(
            &call,
            RuntimeCall::MetaTx(pallet_meta_tx::Call::dispatch { .. })
    )
    || matches!(
            &call,
            RuntimeCall::AssetRewards(pallet_asset_rewards::Call::create_pool { .. })
    )
    || matches!(
            &call,
            RuntimeCall::VoterList(pallet_bags_list::Call::rebag {  .. })
    )
    || matches!(
            &call,
            RuntimeCall::Assets(pallet_assets::Call::set_reserves {  .. })
            | RuntimeCall::PoolAssets(pallet_assets::Call::set_reserves {  .. })
    )
    || matches!(
            &call,
            RuntimeCall::Assets(pallet_assets::Call::transfer_ownership { .. })
    )
    || matches!(
            &call,
            RuntimeCall::Vesting(pallet_vesting::Call::vested_transfer { .. })
    )
    // Kitchensink carries 97 pallets. Without this, the campaign spends nearly
    // all of its budget on pallets the pUSD invariants say nothing about, which
    // is how a `Revive` call ended up in a hang from the first run. Keep only
    // the pallets that can move pUSD, its collateral, or the price that values
    // it.
    || !matches!(
            &call,
            RuntimeCall::Psm(..)
            | RuntimeCall::Vaults(..)
            | RuntimeCall::Assets(..)
            | RuntimeCall::Balances(..)
            | RuntimeCall::Utility(..)
    )
}

fn process_input(accounts: &[AccountId], genesis: &Storage, data: &[u8]) {
    // The first byte picks where in the crash this input starts: 0 is the
    // calm minute, 128 the freefall, 255 the rebound. The rest is extrinsics.
    let (start_tick, data) = match data.split_first() {
        Some((b, rest)) => (price_replay::start_tick(*b), rest),
        None => return,
    };
    // We build the list of extrinsics we will execute
    let mut extrinsic_data = data;
    // Vec<(advance_block, origin, extrinsic)>
    let extrinsics: Vec<(bool, u8, RuntimeCall)> =
        iter::from_fn(|| DecodeLimit::decode_with_depth_limit(64, &mut extrinsic_data).ok())
            .filter(|(_, _, x): &(_, _, RuntimeCall)| {
                !recursively_find_call(x.clone(), call_filter)
            })
            .collect();
    if extrinsics.is_empty() {
        return;
    }

    let mut block: u32 = 1;
    let mut weight: Weight = Weight::zero();
    let mut elapsed: Duration = Duration::ZERO;

    BasicExternalities::execute_with_storage(&mut genesis.clone(), || {
        let initial_total_issuance = TotalIssuance::<Runtime>::get();

        initialize_block(block);
        price_replay::feed(start_tick, block);

        for (advance_block, origin, extrinsic) in extrinsics {
            if advance_block {
                finalize_block(elapsed);

                block += 1;
                weight = Weight::zero();
                elapsed = Duration::ZERO;

                initialize_block(block);
        price_replay::feed(start_tick, block);
            }

            let origin = accounts[origin as usize % accounts.len()].clone();

            // We do not continue if the origin account does not have a free balance
            let account = Account::<Runtime>::get(&origin);
            if account.data.free == 0 {
                #[cfg(not(feature = "fuzzing"))]
                println!("\n    origin {origin:?} does not have free balance, skipping");
                continue;
            }

            #[cfg(not(feature = "fuzzing"))]
            println!("\n    origin:     {origin:?}");
            #[cfg(not(feature = "fuzzing"))]
            println!("    call:       {extrinsic:?}");

            let pre_weight = extrinsic.get_dispatch_info().call_weight;
            let cumulative_weight = weight.saturating_add(pre_weight);
            if cumulative_weight.ref_time() >= 2 * WEIGHT_REF_TIME_PER_SECOND {
                #[cfg(not(feature = "fuzzing"))]
                println!("Extrinsic would exhaust block weight, skipping");
                continue;
            }
            weight = cumulative_weight;

            let now = Instant::now(); // We get the current time for timing purposes.
            let res = extrinsic.dispatch(RuntimeOrigin::signed(origin));
            elapsed += now.elapsed();

            #[cfg(not(feature = "fuzzing"))]
            println!("    result:     {res:?}");

            let actual_weight = res.unwrap_or_else(|e| e.post_info).actual_weight;
            let post_weight = actual_weight.unwrap_or_default();
            assert!(pre_weight.ref_time().saturating_mul(2) >= post_weight.ref_time(), "Pre-dispatch weight ref time ({}) is smaller than post-dispatch weight ref time ({})", pre_weight.ref_time(), post_weight.ref_time());
            assert!(pre_weight.proof_size().saturating_mul(2) >= post_weight.proof_size(), "Pre-dispatch weight proof size ({}) is smaller than post-dispatch weight proof size ({})", pre_weight.proof_size(), post_weight.proof_size());
        }

        finalize_block(elapsed);

        check_invariants(block, initial_total_issuance);
    });
}

mod price_replay {
    //! The price the runtime sees, replayed from the 10 October 2025 crash.
    //!
    //! Static prices left the interesting vaults code unreachable: liquidation,
    //! redistribution and FinalRecovery only run when collateral loses value.
    //! This module walks the recorded crash instead. The series and its
    //! provenance live in [`crate::oct10`]; the genesis vaults that the falling
    //! price pushes under water are opened in `generate_genesis`.
    //!
    //! How the cursor moves:
    //! - An input's first byte picks the starting tick, scaled onto the series:
    //!   byte 0 starts at the calm, ~128 in the freefall, 255 near the end.
    //!   Without it, most inputs (which rarely exceed two blocks) would spend
    //!   the whole campaign at the calm price.
    //! - Each block advances the cursor by `TICKS_PER_BLOCK`, compressing the
    //!   15-minute window into the handful of blocks one input reaches:
    //!   calm to bottom in ~7 blocks at 45 ticks each.
    //! - Past the end, the cursor stays on the last tick.
    //!
    //! How the price reaches the runtime: at the start of every block,
    //! `feed(..)` submits the tick's value from the five oracle member
    //! accounts. Five, because the runtime's `DefaultCombineData` publishes a
    //! median only once `MinimumCount = 5` members have reported. Re-feeding
    //! every block also keeps the values inside the oracle's `ExpiresIn`
    //! window regardless of how far block timestamps jump. `Oracle` calls are
    //! excluded from the fuzzed call set (see `call_filter`), so the replay is
    //! the only writer and the trajectory is authoritative; adversarial feeds
    //! are a separate, future experiment.
    //!
    //! `PUSD_PRICE_SERIES=median|binance` picks the series at startup:
    //! `median` is what a median oracle over the recorded venues would
    //! publish (bottoms at -26%), `binance` the single collapsing venue
    //! (bottoms at $0.98, -74%), which reaches the undercollateralized
    //! FinalRecovery regime that the median path never touches.

    use super::{AccountId, RuntimeOrigin};
    use crate::oct10;

    pub const TICKS_PER_BLOCK: usize = 45;

    /// The oracle key the runtime's vaults adapter reads for native
    /// collateral (`VaultsNativePriceFeedId`).
    const NATIVE_FEED_KEY: u32 = u32::MAX;

    fn series() -> &'static [u128] {
        static CHOICE: std::sync::OnceLock<&'static [u128]> = std::sync::OnceLock::new();
        CHOICE.get_or_init(|| {
            match std::env::var("PUSD_PRICE_SERIES").as_deref() {
                Ok("binance") => &oct10::BINANCE[..],
                _ => &oct10::MEDIAN[..],
            }
        })
    }

    /// Scale an input's first byte onto the series, so every crash phase is
    /// reachable from the first block of an input.
    pub fn start_tick(byte: u8) -> usize {
        byte as usize * series().len() / 256
    }

    /// The vaults math multiplies this price by RAW collateral units to get
    /// RAW stable units, so the oracle inner is not USD-per-token: it must be
    /// scaled by 10^(stable_decimals - native_decimals). Kitchensink's
    /// stablecoin carries 6 decimals and DOLLARS = 10^14, so the USD-per-token
    /// series from [`crate::oct10`] is divided by 10^8. Feeding the unscaled
    /// value prices $37 of collateral as $37M, the ICR bound lands beyond the
    /// branch debt ceiling, and every genesis CR silently loses its meaning;
    /// that is how this constant was found.
    const RAW_UNIT_SCALE: u128 = 100_000_000;

    pub fn price_at(start_tick: usize, block: u32) -> u128 {
        let s = series();
        let idx = start_tick
            .saturating_add((block as usize).saturating_mul(TICKS_PER_BLOCK))
            .min(s.len() - 1);
        s[idx] / RAW_UNIT_SCALE
    }

    /// Feed the tick's price from every oracle member. Errors are ignored on
    /// purpose: a member may be unable to feed in exotic fuzzed states (for
    /// example after its account is reaped), and a missing feed only leaves
    /// the previous tick's price standing, which is a valid market state.
    pub fn feed(start_tick: usize, block: u32) {
        let price = price_at(start_tick, block);
        for i in 0..5u8 {
            let _ = kitchensink_runtime::Oracle::feed_values(
                RuntimeOrigin::signed(AccountId::from([i; 32])),
                vec![(NATIVE_FEED_KEY, price)].try_into().expect("one pair fits the bound"),
            );
        }
    }
}

fn initialize_block(block: u32) {
    #[cfg(not(feature = "fuzzing"))]
    println!("\ninitializing block {block}");

    let pre_digest = Digest {
        logs: vec![DigestItem::PreRuntime(
            BABE_ENGINE_ID,
            PreDigest::SecondaryPlain(SecondaryPlainPreDigest {
                slot: Slot::from(u64::from(block)),
                authority_index: 42,
            })
            .encode(),
        )],
    };

    Executive::initialize_block(&Header::new(
        block,
        H256::default(),
        H256::default(),
        H256::default(),
        pre_digest,
    ));

    #[cfg(not(feature = "fuzzing"))]
    println!("  setting timestamp");
    Timestamp::set(RuntimeOrigin::none(), u64::from(block) * SLOT_DURATION).unwrap();
}

fn finalize_block(elapsed: Duration) {
    #[cfg(not(feature = "fuzzing"))]
    println!("\n  time spent: {elapsed:?}");
    assert!(elapsed.as_secs() <= 2, "block execution took too much time");

    #[cfg(not(feature = "fuzzing"))]
    println!("\n  finalizing block");
    Executive::finalize_block();
}

fn check_invariants(block: u32, initial_total_issuance: Balance) {
    // After execution of all blocks, we run invariants
    let mut counted_free: Balance = 0;
    let mut counted_reserved: Balance = 0;
    for (account, info) in Account::<Runtime>::iter() {
        let consumers = info.consumers;
        let providers = info.providers;
        assert!(!(consumers > 0 && providers == 0), "Invalid c/p state");
        counted_free += info.data.free;
        counted_reserved += info.data.reserved;
        let max_lock: Balance = Balances::locks(&account)
            .iter()
            .map(|l| l.amount)
            .max()
            .unwrap_or_default();
        assert_eq!(
            max_lock, info.data.frozen,
            "Max lock should be equal to frozen balance"
        );
        let sum_holds: Balance = Holds::<Runtime>::get(&account)
            .iter()
            .map(|l| l.amount)
            .sum();
        assert!(
            sum_holds <= info.data.reserved,
            "Sum of all holds ({sum_holds}) should be less than or equal to reserved balance {}",
            info.data.reserved
        );
    }
    let total_issuance = TotalIssuance::<Runtime>::get();
    let counted_issuance = counted_free + counted_reserved;
    assert_eq!(total_issuance, counted_issuance);
    assert!(total_issuance <= initial_total_issuance);
    // We run all developer-defined integrity tests
    AllPalletsWithSystem::integrity_test();
    AllPalletsWithSystem::try_state(block, TryStateSelect::All).unwrap();
}
