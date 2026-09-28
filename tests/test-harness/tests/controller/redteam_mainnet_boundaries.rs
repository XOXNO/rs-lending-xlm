use common::constants::BPS;
use common::math::fp_core::mul_div_ceil;
use common::types::SeizeMode;
use controller::constants::{RAY, WAD};
use test_harness::errors::codes::HEALTH_FACTOR_TOO_HIGH;
use test_harness::mainnet::{
    mainnet_market, LiquidationObservation, LiquidationVector, MainnetSpoke, SeizedLeg,
    VectorOutcome,
};
use test_harness::{assert_contract_error, ALICE, CAROL, LIQUIDATOR};

const HF_ONE_EDGE: LiquidationVector = LiquidationVector {
    name: "A5.1 s1 XLM->USDC HF=1 edge",
    spoke: 1,
    open_prices: &[],
    collateral: &[("XLM", 784_628_050_444)],
    debt: &[("USDC", 100_000_930_270)],
    prices: &[("XLM", 163_396_055_153_190_000)],
    hf_wad: 999_999_999_999_964_026,
    bonus_bps: 1073,
    repay_usd_wad: 5_239_149_720_960_426_859_251,
    repaid: &[("USDC", 52_391_984_592)],
    seized: &[SeizedLeg {
        asset: "XLM",
        gross: 355_045_933_060,
        to_liquidator: 350_917_357_757,
        fee: 4_128_575_303,
    }],
    post_collateral_usd_wad: 7_019_202_334_490_020_468_205,
    post_weighted_usd_wad: 5_474_977_820_902_215_965_199,
    post_debt_usd_wad: 4_760_850_279_036_778_892_359,
    outcome: VectorOutcome::Open {
        hf_wad: 1_150_000_000_002_083_711,
    },
};

const BAND_TARGET_SIDE: LiquidationVector = LiquidationVector {
    name: "A5.2 s1 band start, target side",
    spoke: 1,
    open_prices: &[],
    collateral: &[("XLM", 784_628_050_444)],
    debt: &[("USDC", 100_000_930_270)],
    prices: &[("XLM", 138_919_326_091_250_000)],
    hf_wad: 850_200_000_000_017_531,
    bonus_bps: 900,
    repay_usd_wad: 9_999_999_999_997_205_751_609,
    repaid: &[("USDC", 100_000_930_270)],
    seized: &[SeizedLeg {
        asset: "XLM",
        gross: 784_628_050_443,
        to_liquidator: 776_853_754_164,
        fee: 7_774_296_279,
    }],
    post_collateral_usd_wad: 13_891_932_609,
    post_weighted_usd_wad: 10_835_707_435,
    post_debt_usd_wad: 0,
    outcome: VectorOutcome::DebtFree,
};

const BAND_BAND_SIDE: LiquidationVector = LiquidationVector {
    name: "A5.2 s1 band start, band side",
    spoke: 1,
    open_prices: &[],
    collateral: &[("XLM", 784_628_050_444)],
    debt: &[("USDC", 100_000_930_270)],
    prices: &[("XLM", 138_919_326_091_240_000)],
    hf_wad: 850_199_999_999_956_330,
    bonus_bps: 899,
    repay_usd_wad: 9_999_999_999_997_205_751_609,
    repaid: &[("USDC", 100_000_930_270)],
    seized: &[SeizedLeg {
        asset: "XLM",
        gross: 784_556_066_219,
        to_liquidator: 776_790_408_047,
        fee: 7_765_658_172,
    }],
    post_collateral_usd_wad: 1_000_000_002_620_019_069,
    post_weighted_usd_wad: 780_000_002_043_614_873,
    post_debt_usd_wad: 0,
    outcome: VectorOutcome::DebtFree,
};

const C_GE_D: LiquidationVector = LiquidationVector {
    name: "A5.3 s1 C>=D band at bonus 0",
    spoke: 1,
    open_prices: &[],
    collateral: &[("XLM", 784_628_050_444)],
    debt: &[("USDC", 100_000_930_270)],
    prices: &[("XLM", 127_448_923_019_500_000)],
    hf_wad: 780_000_000_000_044_157,
    bonus_bps: 0,
    repay_usd_wad: 9_999_999_999_997_205_751_609,
    repaid: &[("USDC", 100_000_930_270)],
    seized: &[SeizedLeg {
        asset: "XLM",
        gross: 784_628_050_443,
        to_liquidator: 784_628_050_443,
        fee: 0,
    }],
    post_collateral_usd_wad: 12_744_892_302,
    post_weighted_usd_wad: 9_941_015_994,
    post_debt_usd_wad: 0,
    outcome: VectorOutcome::DebtFree,
};

const C_LT_D: LiquidationVector = LiquidationVector {
    name: "A5.3 s1 C<D insolvent",
    spoke: 1,
    open_prices: &[],
    collateral: &[("XLM", 784_628_050_444)],
    debt: &[("USDC", 100_000_930_270)],
    prices: &[("XLM", 127_448_923_019_490_000)],
    hf_wad: 779_999_999_999_982_956,
    bonus_bps: 900,
    repay_usd_wad: 9_174_311_926_525_877_553_237,
    repaid: &[("USDC", 91_743_972_724)],
    seized: &[SeizedLeg {
        asset: "XLM",
        gross: 784_628_050_444,
        to_liquidator: 776_853_754_164,
        fee: 7_774_296_280,
    }],
    post_collateral_usd_wad: 0,
    post_weighted_usd_wad: 0,
    post_debt_usd_wad: 825_688_073_471_328_198_372,
    outcome: VectorOutcome::BadDebt {
        socialized: &[("USDC", 8_256_957_546)],
    },
};

const DUST_PROMOTED: LiquidationVector = LiquidationVector {
    name: "A5.4 s1 dust promoted",
    spoke: 1,
    open_prices: &[("XLM", 300_000_000_000_000_000)],
    collateral: &[("XLM", 801_060_028)],
    debt: &[("USDC", 136_828_602)],
    prices: &[("XLM", 212_414_871_690_620_000)],
    hf_wad: 969_999_999_471_783_030,
    bonus_bps: 1107,
    repay_usd_wad: 13_682_732_913_636_700_981,
    repaid: &[("USDC", 136_828_602)],
    seized: &[SeizedLeg {
        asset: "XLM",
        gross: 715_458_918,
        to_liquidator: 706_902_012,
        fee: 8_556_906,
    }],
    post_collateral_usd_wad: 1_818_294_879_722_464_859,
    post_weighted_usd_wad: 1_418_270_006_183_522_589,
    post_debt_usd_wad: 0,
    outcome: VectorOutcome::DebtFree,
};

const DUST_NOT_PROMOTED: LiquidationVector = LiquidationVector {
    name: "A5.4 s1 dust not promoted",
    spoke: 1,
    open_prices: &[("XLM", 300_000_000_000_000_000)],
    collateral: &[("XLM", 801_060_034)],
    debt: &[("USDC", 136_828_603)],
    prices: &[("XLM", 212_414_871_690_620_000)],
    hf_wad: 969_999_999_647_995_074,
    bonus_bps: 1107,
    repay_usd_wad: 8_682_733_027_137_042_258,
    repaid: &[("USDC", 86_828_138)],
    seized: &[SeizedLeg {
        asset: "XLM",
        gross: 454_013_012,
        to_liquidator: 448_583_005,
        fee: 5_430_007,
    }],
    post_collateral_usd_wad: 7_371_794_864_874_177_633,
    post_weighted_usd_wad: 5_749_999_994_601_858_553,
    post_debt_usd_wad: 4_999_999_986_498_728_462,
    outcome: VectorOutcome::Open {
        hf_wad: 1_150_000_002_025_664_169,
    },
};

const HF_ONE_USST_USDC: LiquidationVector = LiquidationVector {
    name: "A5.8 on-chain 4 USST->USDC HF=1 edge",
    spoke: 4,
    open_prices: &[
        ("USST", 1_120_000_000_000_000_000),
        ("USDC", 950_000_000_000_000_000),
    ],
    collateral: &[("USST", 12_238_658_419_943_914_949_086)],
    debt: &[("USDC", 100_000_930_270)],
    prices: &[
        ("USST", 961_274_142_856_870_000),
        ("USDC", 999_990_697_386_260_000),
    ],
    hf_wad: 999_999_999_999_995_575,
    bonus_bps: 615,
    repay_usd_wad: 1_698_874_495_679_754_327_801,
    repaid: &[("USDC", 16_988_902_998)],
    seized: &[SeizedLeg {
        asset: "USST",
        gross: 1_876_005_185_996_739_996_816,
        to_liquidator: 1_865_136_196_930_437_640_735,
        fee: 10_868_989_066_302_356_081,
    }],
    post_collateral_usd_wad: 9_961_350_605_185_542_552_140,
    post_weighted_usd_wad: 8_467_148_014_407_711_169_318,
    post_debt_usd_wad: 8_301_125_504_317_451_423_809,
    outcome: VectorOutcome::Open {
        hf_wad: 1_020_000_000_000_471_106,
    },
};

const DUST_USST_USDC: LiquidationVector = LiquidationVector {
    name: "A5.8 on-chain 4 USST->USDC dust promoted",
    spoke: 4,
    open_prices: &[
        ("USST", 1_120_000_000_000_000_000),
        ("USDC", 950_000_000_000_000_000),
    ],
    collateral: &[("USST", 10_975_657_958_008_506_246)],
    debt: &[("USDC", 97_077_449)],
    prices: &[
        ("USST", 1_009_337_850_000_000_000),
        ("USDC", 999_990_697_386_260_000),
    ],
    hf_wad: 969_999_999_999_999_999,
    bonus_bps: 787,
    repay_usd_wad: 9_707_654_592_598_908_845,
    repaid: &[("USDC", 97_077_449)],
    seized: &[SeizedLeg {
        asset: "USST",
        gross: 10_374_768_972_585_782_819,
        to_liquidator: 10_299_076_535_565_063_332,
        fee: 75_692_437_020_719_487,
    }],
    post_collateral_usd_wad: 606_499_996_635_253_005,
    post_weighted_usd_wad: 515_524_997_139_965_053,
    post_debt_usd_wad: 0,
    outcome: VectorOutcome::DebtFree,
};

const NO_DUST_USST_USDC: LiquidationVector = LiquidationVector {
    name: "A5.8 on-chain 4 USST->USDC dust not promoted",
    spoke: 4,
    open_prices: &[
        ("USST", 1_120_000_000_000_000_000),
        ("USDC", 950_000_000_000_000_000),
    ],
    collateral: &[("USST", 10_975_658_071_069_346_544)],
    debt: &[("USDC", 97_077_450)],
    prices: &[
        ("USST", 1_009_337_850_000_000_000),
        ("USDC", 999_990_697_386_260_000),
    ],
    hf_wad: 969_999_999_999_999_999,
    bonus_bps: 787,
    repay_usd_wad: 4_707_654_706_099_250_123,
    repaid: &[("USDC", 47_076_985)],
    seized: &[SeizedLeg {
        asset: "USST",
        gross: 5_031_166_850_098_072_819,
        to_liquidator: 4_994_460_367_192_471_788,
        fee: 36_706_482_905_601_031,
    }],
    post_collateral_usd_wad: 5_999_999_988_319_020_333,
    post_weighted_usd_wad: 5_099_999_990_071_167_283,
    post_debt_usd_wad: 4_999_999_986_498_728_462,
    outcome: VectorOutcome::Open {
        hf_wad: 1_020_000_000_768_492_852,
    },
};

const HF_ONE_DEJTRSY_XLM: LiquidationVector = LiquidationVector {
    name: "A5.8 on-chain 3 DEJTRSY->XLM HF=1 edge",
    spoke: 3,
    open_prices: &[
        ("DEJTRSY", 1_200_000_000_000_000_000),
        ("XLM", 170_000_000_000_000_000),
    ],
    collateral: &[("DEJTRSY", 14_181_380_629_274_946_211_950)],
    debt: &[("XLM", 470_776_830_285)],
    prices: &[
        ("DEJTRSY", 1_007_357_087_377_680_000),
        ("XLM", 212_414_871_690_620_000),
    ],
    hf_wad: 999_999_999_999_995_612,
    bonus_bps: 932,
    repay_usd_wad: 3_898_534_151_175_569_422_818,
    repaid: &[("XLM", 183_533_955_045)],
    seized: &[SeizedLeg {
        asset: "DEJTRSY",
        gross: 4_230_751_525_419_369_245_878,
        to_liquidator: 4_187_468_754_599_491_615_394,
        fee: 43_282_770_819_877_630_484,
    }],
    post_collateral_usd_wad: 10_023_836_751_635_528_081_191,
    post_weighted_usd_wad: 7_016_685_726_144_869_656_833,
    post_debt_usd_wad: 6_101_465_848_814_936_853_825,
    outcome: VectorOutcome::Open {
        hf_wad: 1_150_000_000_001_260_725,
    },
};

const DUST_DEJTRSY_XLM: LiquidationVector = LiquidationVector {
    name: "A5.8 on-chain 3 DEJTRSY->XLM dust promoted",
    spoke: 3,
    open_prices: &[
        ("DEJTRSY", 1_200_000_000_000_000_000),
        ("XLM", 170_000_000_000_000_000),
    ],
    collateral: &[("DEJTRSY", 12_683_312_753_996_049_029)],
    debt: &[("XLM", 447_089_797)],
    prices: &[
        ("DEJTRSY", 1_037_577_800_000_000_000),
        ("XLM", 212_414_871_690_620_000),
    ],
    hf_wad: 969_999_999_999_999_999,
    bonus_bps: 998,
    repay_usd_wad: 9_496_852_186_394_034_260,
    repaid: &[("XLM", 447_089_797)],
    seized: &[SeizedLeg {
        asset: "DEJTRSY",
        gross: 10_066_366_141_022_060_109,
        to_liquidator: 9_956_750_937_435_153_225,
        fee: 109_615_203_586_906_884,
    }],
    post_collateral_usd_wad: 2_715_285_709_407_002_881,
    post_weighted_usd_wad: 1_900_699_996_584_902_016,
    post_debt_usd_wad: 0,
    outcome: VectorOutcome::DebtFree,
};

const NO_DUST_DEJTRSY_XLM: LiquidationVector = LiquidationVector {
    name: "A5.8 on-chain 3 DEJTRSY->XLM dust not promoted",
    spoke: 3,
    open_prices: &[
        ("DEJTRSY", 1_200_000_000_000_000_000),
        ("XLM", 170_000_000_000_000_000),
    ],
    collateral: &[("DEJTRSY", 12_683_312_782_364_651_576)],
    debt: &[("XLM", 447_089_798)],
    prices: &[
        ("DEJTRSY", 1_037_577_800_000_000_000),
        ("XLM", 212_414_871_690_620_000),
    ],
    hf_wad: 969_999_999_999_999_999,
    bonus_bps: 998,
    repay_usd_wad: 4_496_852_210_667_180_213,
    repaid: &[("XLM", 211_701_383)],
    seized: &[SeizedLeg {
        asset: "DEJTRSY",
        gross: 4_766_522_627_307_335_217,
        to_liquidator: 4_714_618_758_436_056_215,
        fee: 51_903_868_871_279_002,
    }],
    post_collateral_usd_wad: 8_214_285_712_146_029_182,
    post_weighted_usd_wad: 5_749_999_998_502_220_426,
    post_debt_usd_wad: 4_999_999_996_968_341_217,
    outcome: VectorOutcome::Open {
        hf_wad: 1_150_000_000_397_725_605,
    },
};

const HF_ONE_XAUM_USDC: LiquidationVector = LiquidationVector {
    name: "A5.8 on-chain 7 XAUM->USDC HF=1 edge",
    spoke: 7,
    open_prices: &[("XAUM", 6_300_000_000_000_000_000_000)],
    collateral: &[("XAUM", 1_654_285_362)],
    debt: &[("USDC", 40_000_372_108)],
    prices: &[("XAUM", 3_223_950_024_489_089_970_000)],
    hf_wad: 999_999_999_999_999_999,
    bonus_bps: 952,
    repay_usd_wad: 1_825_928_180_237_407_491_035,
    repaid: &[("USDC", 18_259_451_663)],
    seized: &[SeizedLeg {
        asset: "XAUM",
        gross: 620_281_495,
        to_liquidator: 614_889_713,
        fee: 5_391_782,
    }],
    post_collateral_usd_wad: 3_333_576_792_336_463_728_291,
    post_weighted_usd_wad: 2_500_182_594_252_347_796_217,
    post_debt_usd_wad: 2_174_071_819_761_474_809_609,
    outcome: VectorOutcome::Open {
        hf_wad: 1_150_000_000_702_208_524,
    },
};

const DUST_XAUM_USDC: LiquidationVector = LiquidationVector {
    name: "A5.8 on-chain 7 XAUM->USDC dust promoted",
    spoke: 7,
    open_prices: &[("XAUM", 6_300_000_000_000_000_000_000)],
    collateral: &[("XAUM", 3_440_645)],
    debt: &[("USDC", 111_497_521)],
    prices: &[("XAUM", 4_191_135_031_763_473_690_000)],
    hf_wad: 969_999_723_462_431_534,
    bonus_bps: 982,
    repay_usd_wad: 11_149_648_378_162_916_946,
    repaid: &[("USDC", 111_497_521)],
    seized: &[SeizedLeg {
        asset: "XAUM",
        gross: 2_921_534,
        to_liquidator: 2_895_410,
        fee: 26_124,
    }],
    post_collateral_usd_wad: 2_175_664_297_473_768_591,
    post_weighted_usd_wad: 1_631_748_223_105_326_442,
    post_debt_usd_wad: 0,
    outcome: VectorOutcome::DebtFree,
};

const NO_DUST_XAUM_USDC: LiquidationVector = LiquidationVector {
    name: "A5.8 on-chain 7 XAUM->USDC dust not promoted",
    spoke: 7,
    open_prices: &[("XAUM", 6_300_000_000_000_000_000_000)],
    collateral: &[("XAUM", 3_440_646)],
    debt: &[("USDC", 111_497_522)],
    prices: &[("XAUM", 4_191_135_031_763_473_690_000)],
    hf_wad: 969_999_996_686_489_656,
    bonus_bps: 982,
    repay_usd_wad: 6_149_645_391_692_096_326,
    repaid: &[("USDC", 61_497_026)],
    seized: &[SeizedLeg {
        asset: "XAUM",
        gross: 1_611_387,
        to_liquidator: 1_596_979,
        fee: 14_408,
    }],
    post_collateral_usd_wad: 7_666_671_477_068_620_119,
    post_weighted_usd_wad: 5_750_003_607_801_465_088,
    post_debt_usd_wad: 5_000_003_086_469_890_359,
    outcome: VectorOutcome::Open {
        hf_wad: 1_150_000_011_672_211_029,
    },
};

const HF_ONE_XLM_SOLVBTC: LiquidationVector = LiquidationVector {
    name: "A5.8 on-chain 1 XLM->SolvBTC HF=1 edge",
    spoke: 1,
    open_prices: &[("XLM", 300_000_000_000_000_000)],
    collateral: &[("XLM", 784_628_050_475)],
    debt: &[("SolvBTC", 12_009_423)],
    prices: &[("XLM", 163_396_043_774_560_000)],
    hf_wad: 999_999_999_999_971_567,
    bonus_bps: 1073,
    repay_usd_wad: 5_239_149_759_391_545_444_672,
    repaid: &[("SolvBTC", 6_291_917)],
    seized: &[SeizedLeg {
        asset: "XLM",
        gross: 355_045_960_389,
        to_liquidator: 350_917_384_768,
        fee: 4_128_575_621,
    }],
    post_collateral_usd_wad: 7_019_201_399_645_903_339_501,
    post_weighted_usd_wad: 5_474_977_091_723_804_604_810,
    post_debt_usd_wad: 4_760_849_544_617_279_189_981,
    outcome: VectorOutcome::Open {
        hf_wad: 1_150_000_024_242_298_029,
    },
};

const DUST_XLM_SOLVBTC: LiquidationVector = LiquidationVector {
    name: "A5.8 on-chain 1 XLM->SolvBTC dust promoted",
    spoke: 1,
    open_prices: &[("XLM", 300_000_000_000_000_000)],
    collateral: &[("XLM", 801_051_551)],
    debt: &[("SolvBTC", 16_432)],
    prices: &[("XLM", 212_414_871_690_620_000)],
    hf_wad: 969_999_998_895_634_383,
    bonus_bps: 1107,
    repay_usd_wad: 13_682_588_127_961_935_090,
    repaid: &[("SolvBTC", 16_432)],
    seized: &[SeizedLeg {
        asset: "XLM",
        gross: 715_451_348,
        to_liquidator: 706_894_532,
        fee: 8_556_816,
    }],
    post_collateral_usd_wad: 1_818_275_613_693_602_520,
    post_weighted_usd_wad: 1_418_254_978_681_009_964,
    post_debt_usd_wad: 0,
    outcome: VectorOutcome::DebtFree,
};

const NO_DUST_XLM_SOLVBTC: LiquidationVector = LiquidationVector {
    name: "A5.8 on-chain 1 XLM->SolvBTC dust not promoted",
    spoke: 1,
    open_prices: &[("XLM", 300_000_000_000_000_000)],
    collateral: &[("XLM", 801_100_301)],
    debt: &[("SolvBTC", 16_433)],
    prices: &[("XLM", 212_414_871_690_620_000)],
    hf_wad: 969_999_999_518_220_917,
    bonus_bps: 1107,
    repay_usd_wad: 8_683_180_927_360_458_807,
    repaid: &[("SolvBTC", 10_428)],
    seized: &[SeizedLeg {
        asset: "XLM",
        gross: 454_036_432,
        to_liquidator: 448_606_145,
        fee: 5_430_287,
    }],
    post_collateral_usd_wad: 7_372_152_720_208_514_821,
    post_weighted_usd_wad: 5_750_279_121_762_641_559,
    post_debt_usd_wad: 5_000_239_880_015_300_647,
    outcome: VectorOutcome::Open {
        hf_wad: 1_150_000_651_917_732_755,
    },
};

fn check(v: &LiquidationVector) -> LiquidationObservation {
    let mut m = MainnetSpoke::open_vector(v, ALICE);
    m.assert_transfer_vector(LIQUIDATOR, ALICE, v)
}

fn check_hf_one_edge(v: &LiquidationVector, above: (&str, i128), above_hf: i128) {
    let mut m = MainnetSpoke::build(v.spoke);
    m.set_prices(v.open_prices);
    m.open_position(ALICE, v.collateral, v.debt);
    m.set_prices(v.prices);
    m.set_price(above.0, above.1);
    assert_eq!(m.health_factor_raw(ALICE), above_hf, "{}: HF above", v.name);
    for mode in [SeizeMode::Transfer, SeizeMode::Credit(0)] {
        let res = m.try_liquidate(LIQUIDATOR, ALICE, v.debt, mode);
        assert_contract_error(res, HEALTH_FACTOR_TOO_HIGH);
    }
    m.set_prices(v.prices);
    m.assert_transfer_vector(LIQUIDATOR, ALICE, v);
}

fn check_dust_pair(promoted: &LiquidationVector, partial: &LiquidationVector) {
    assert_eq!(promoted.debt[0].1 + 1, partial.debt[0].1);
    assert!(promoted.seized[0].gross <= promoted.collateral[0].1);
    let full = check(promoted);
    assert!(full.debt_free);
    let part = check(partial);
    assert!(!part.debt_free);
    let unit_usd = |asset: &str| {
        let m = mainnet_market(asset);
        let price = partial
            .prices
            .iter()
            .find(|(a, _)| *a == asset)
            .map_or(m.price_wad - m.price_wad % 10_000, |(_, p)| *p);
        let unit = 10i128.pow(m.decimals);
        (price + unit - 1) / unit
    };
    let tolerance = unit_usd(partial.collateral[0].0).max(unit_usd(partial.debt[0].0));
    assert!(
        (partial.post_debt_usd_wad - 5 * WAD).abs() <= tolerance,
        "{}: post debt {} is not within one native unit ({tolerance}) of $5",
        partial.name,
        partial.post_debt_usd_wad
    );
    assert_eq!(promoted.bonus_bps, partial.bonus_bps);
}

#[test]
fn redteam_boundary_a5_1_hf_one_edge_s1_xlm_usdc() {
    check_hf_one_edge(
        &HF_ONE_EDGE,
        ("XLM", 163_396_055_153_200_000),
        1_000_000_000_000_025_227,
    );
}

#[test]
fn redteam_boundary_a5_2_band_start_is_continuous_s1() {
    let target = check(&BAND_TARGET_SIDE);
    let band = check(&BAND_BAND_SIDE);
    assert_eq!((target.bonus_bps, band.bonus_bps), (900, 899));
    let m = MainnetSpoke::build(1);
    let bound = mul_div_ceil(
        &m.t.env,
        BAND_TARGET_SIDE.repay_usd_wad,
        10_000_000,
        BPS * 138_919_326_091_240_000,
    ) + 1;
    assert_eq!(bound, 71_984_226);
    let step = target.liquidator_delta["XLM"] - band.liquidator_delta["XLM"];
    assert_eq!(step, 63_346_117);
    assert!(step <= bound);
}

#[test]
fn redteam_boundary_a5_3_c_eq_d_flip_s1() {
    let solvent = check(&C_GE_D);
    assert_eq!(solvent.bonus_bps, 0);
    assert_eq!(solvent.bad_debt_event, None);
    let insolvent = check(&C_LT_D);
    assert_eq!(insolvent.bonus_bps, 900);
    assert_eq!(
        insolvent.bad_debt_event,
        Some((825_688_073_471_328_198_372, 0))
    );
}

#[test]
fn redteam_boundary_a5_4_dust_promotion_edge_s1() {
    check_dust_pair(&DUST_PROMOTED, &DUST_NOT_PROMOTED);
    assert_eq!(
        DUST_PROMOTED.repay_usd_wad - DUST_NOT_PROMOTED.repay_usd_wad,
        4_999_999_886_499_658_723
    );
}

const GATE_XLM: i128 = 2_824_660_981;
const GATE_USDC: i128 = 618_562_455;
const XLM_SHARE: i128 = 10i128.pow(20);

fn cleanup_gate_book() -> MainnetSpoke {
    let mut m = MainnetSpoke::build(1);
    m.set_price("XLM", 300_000_000_000_000_000);
    m.open_position(ALICE, &[("XLM", GATE_XLM)], &[("USDC", GATE_USDC)]);
    m.set_price("XLM", 212_414_871_690_620_000);
    assert_eq!(m.health_factor_raw(ALICE), 756_600_000_149_782_791);
    m.supply(CAROL, "USDC", 10_000_000_000);
    assert_eq!(m.pool_state("USDC").supplied, 10_000_000_000 * XLM_SHARE);
    m
}

#[test]
fn redteam_boundary_a5_5_cleanup_gate_edge_s1() {
    let mut above = cleanup_gate_book();
    let kept = above
        .observe_liquidation(
            LIQUIDATOR,
            ALICE,
            &[("USDC", 504_591_849)],
            SeizeMode::Transfer,
        )
        .expect("gate minus one");
    assert_eq!(kept.bonus_bps, 900);
    assert_eq!(kept.repaid_usd_wad, 50_458_715_497_693_240_059);
    assert_eq!(kept.shares_burned["XLM"], 2_589_272_561 * XLM_SHARE);
    assert_eq!(kept.liquidator_delta["XLM"], 2_563_617_384);
    assert_eq!(kept.revenue_delta["XLM"], 25_655_177 * XLM_SHARE);
    assert_eq!(kept.bad_debt_event, None);
    assert!(!kept.debt_free);
    let post = kept.post.expect("account stays");
    assert_eq!(
        (
            post.total_collateral,
            post.weighted_collateral,
            post.total_debt,
            post.health_factor
        ),
        (
            5_000_000_103_175_777_062,
            3_900_000_080_477_106_108,
            11_396_954_577_547_466_828,
            342_196_685_433_869_196
        )
    );
    assert!(post.total_collateral > 5 * WAD);
    above.t.assert_spoke_usage_matches_positions();

    let mut gate = cleanup_gate_book();
    let usdc_index = gate.pool_state("USDC").supply_index;
    let cleaned = gate
        .observe_liquidation(
            LIQUIDATOR,
            ALICE,
            &[("USDC", 504_591_850)],
            SeizeMode::Transfer,
        )
        .expect("gate");
    assert_eq!(cleaned.repaid_usd_wad, 50_458_715_597_692_309_798);
    assert_eq!(cleaned.liquidator_delta["XLM"], 2_563_617_389);
    assert_eq!(cleaned.shares_burned["XLM"], GATE_XLM * XLM_SHARE);
    assert_eq!(
        cleaned.revenue_delta["XLM"],
        (25_655_177 + 235_388_415) * XLM_SHARE
    );
    assert_eq!(
        cleaned.bad_debt_event,
        Some((11_396_954_477_548_397_089, 4_999_999_996_968_341_217))
    );
    assert!(cleaned.debt_free);
    assert_eq!(cleaned.borrowed_burned["USDC"], GATE_USDC * XLM_SHARE);
    assert_eq!(usdc_index, RAY);
    assert_eq!(
        gate.pool_state("USDC").supply_index,
        988_602_939_500_000_000_000_000_000
    );
    gate.t.assert_spoke_usage_matches_positions();

    let mut one_shot = cleanup_gate_book();
    let full = one_shot
        .observe_liquidation(
            LIQUIDATOR,
            ALICE,
            &[("USDC", 10_000_000_000)],
            SeizeMode::Transfer,
        )
        .expect("full quote");
    assert_eq!(full.liquidator_delta["USDC"], 10_000_000_000 - 550_463_836);
    assert_eq!(full.shares_burned["XLM"], GATE_XLM * XLM_SHARE);
    assert_eq!(full.liquidator_delta["XLM"], 2_796_673_515);
    assert_eq!(full.revenue_delta["XLM"], 27_987_466 * XLM_SHARE);
    assert_eq!(full.bad_debt_event, Some((6_809_798_550_485_121_558, 0)));
    assert_eq!(
        one_shot.pool_state("USDC").supply_index,
        993_190_138_100_000_000_000_000_000
    );
    assert_eq!(
        cleaned.bad_debt_event.unwrap().0 - full.bad_debt_event.unwrap().0,
        4_587_155_927_063_275_531
    );
    one_shot.t.assert_spoke_usage_matches_positions();
}

#[test]
fn redteam_boundary_a5_8_usst_18_decimals_to_usdc() {
    check_hf_one_edge(
        &HF_ONE_USST_USDC,
        ("USST", 961_274_142_856_880_000),
        1_000_000_000_000_005_978,
    );
    check_dust_pair(&DUST_USST_USDC, &NO_DUST_USST_USDC);
}

#[test]
fn redteam_boundary_a5_8_dejtrsy_18_decimals_to_xlm() {
    check_hf_one_edge(
        &HF_ONE_DEJTRSY_XLM,
        ("DEJTRSY", 1_007_357_087_377_690_000),
        1_000_000_000_000_005_539,
    );
    check_dust_pair(&DUST_DEJTRSY_XLM, &NO_DUST_DEJTRSY_XLM);
}

#[test]
fn redteam_boundary_a5_8_xaum_9_decimals_to_usdc() {
    check_hf_one_edge(
        &HF_ONE_XAUM_USDC,
        ("XAUM", 3_223_950_024_489_089_980_000),
        1_000_000_000_000_000_002,
    );
    check_dust_pair(&DUST_XAUM_USDC, &NO_DUST_XAUM_USDC);
}

#[test]
fn redteam_boundary_a5_8_xlm_to_solvbtc_8_decimals() {
    check_hf_one_edge(
        &HF_ONE_XLM_SOLVBTC,
        ("XLM", 163_396_043_774_570_000),
        1_000_000_000_000_032_768,
    );
    check_dust_pair(&DUST_XLM_SOLVBTC, &NO_DUST_XLM_SOLVBTC);
}
