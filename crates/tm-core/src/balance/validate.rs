use core::fmt;

use tm_numeric::BigNum;

use super::derived::{self, BalanceDerived, cap_u32, cost_table, eval_money, eval_u64};
use super::{BALANCE_FORMAT_VERSION, BalanceSpec, ValueFormula};
use crate::ids::{CompanyLevel, Level};
use crate::param::{Feature, ParamKey, RebirthUpgrade};
use crate::time::POS_UNITS_PER_TILE;

/// 表の読み込み・検証のエラー。
#[derive(Clone, Debug, PartialEq)]
pub enum BalanceError {
    /// RON の構文・型の誤り。
    Parse {
        /// `ron` のエラーの文字列。
        message: String,
    },
    /// 検証(10 §4.4)に通らなかった。
    Invalid {
        /// 検査の ID(`"BV-06"` など)。
        rule: &'static str,
        /// 何が違反したか。
        detail: String,
    },
}

impl fmt::Display for BalanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BalanceError::Parse { message } => write!(f, "balance.ron の読み込みに失敗: {message}"),
            BalanceError::Invalid { rule, detail } => write!(f, "{rule}: {detail}"),
        }
    }
}

impl std::error::Error for BalanceError {}

fn invalid(rule: &'static str, detail: String) -> BalanceError {
    BalanceError::Invalid { rule, detail }
}

/// `Level` は 255 で `next` が panic するので、レベルの最大は 254 まで。
const MAX_LEVEL_LIMIT: u32 = 254;
/// 経路のノード数の上限(BV-09)。192 を超えるグラフは 12 の予備案 1 で計算する(RT-D-16)。
const MAX_ROUTE_NODES: u64 = 256;
/// マップのマス数の上限(BV-20。10 TD-42 の `TileBits`)。
const MAX_MAP_TILES: u32 = 1024;

const FEATURES: [Feature; 3] = [Feature::LineExtend, Feature::BulkBuy, Feature::Transfer];

/// 検証して前計算する。検査の順は構造(BV-01〜05、07、08、12、13、15、16、18〜22)→ 表(BV-10、11、19)→ 表を使う検査(BV-06、09、14、17)。
pub(super) fn validate(spec: &BalanceSpec) -> Result<BalanceDerived, BalanceError> {
    check_structure(spec)?;
    let d = derive(spec)?;
    check_tables(spec, &d)?;
    Ok(d)
}

fn check_structure(spec: &BalanceSpec) -> Result<(), BalanceError> {
    if spec.format_version != BALANCE_FORMAT_VERSION {
        return Err(invalid(
            "BV-01",
            format!("format_version = {}", spec.format_version),
        ));
    }

    check_rebirth(spec)?;

    let keys: Vec<ParamKey> = spec.params.iter().map(|p| p.key).collect();
    if keys != ParamKey::ALL {
        return Err(invalid("BV-02", format!("params の並び = {keys:?}")));
    }

    for p in &spec.params {
        if p.base_yen < 1 || p.cost_ratio < 1 {
            return Err(invalid(
                "BV-03",
                format!(
                    "{:?}: base_yen = {}、cost_ratio = {}",
                    p.key, p.base_yen, p.cost_ratio
                ),
            ));
        }
    }

    for p in &spec.params {
        let (v0, ratio_ok) = match p.value {
            ValueFormula::Linear { v0, .. } => (v0, true),
            ValueFormula::Geometric { v0, ratio } => (v0, ratio >= 1),
        };
        if !ratio_ok {
            return Err(invalid("BV-04", format!("{:?}: ratio が 0", p.key)));
        }
        let needs_positive = matches!(
            p.key,
            ParamKey::T1Speed
                | ParamKey::T3Boarding
                | ParamKey::R1MaxTrains
                | ParamKey::R2LineSlots
                | ParamKey::R3Attractiveness
        );
        if needs_positive && v0 < 1 {
            return Err(invalid("BV-04", format!("{:?}: v0 が 0", p.key)));
        }
    }

    let company = &spec.company;
    let cap_max = cap_u32(company, u32::from(company.max_level));
    if cap_max > MAX_LEVEL_LIMIT {
        return Err(invalid(
            "BV-05",
            format!("cap(max_level) = {cap_max} が {MAX_LEVEL_LIMIT} を超える"),
        ));
    }
    for p in &spec.params {
        if p.unlock > company.max_level {
            return Err(invalid(
                "BV-05",
                format!("{:?}: unlock = {}", p.key, p.unlock),
            ));
        }
        if let Some(m) = p.intrinsic_max
            && u32::from(m) > cap_max
        {
            return Err(invalid(
                "BV-05",
                format!(
                    "{:?}: intrinsic_max = {m} が cap(max_level) = {cap_max} を超える",
                    p.key
                ),
            ));
        }
    }
    for f in &spec.features {
        if f.unlock > company.max_level {
            return Err(invalid(
                "BV-05",
                format!("{:?}: unlock = {}", f.feature, f.unlock),
            ));
        }
    }

    if company.gauge_num < 1 || company.gauge_den < 1 || company.gauge_num >= company.gauge_den {
        return Err(invalid(
            "BV-07",
            format!(
                "gauge_num = {}、gauge_den = {}",
                company.gauge_num, company.gauge_den
            ),
        ));
    }

    let map = &spec.map;
    let place_w = i32::from(map.width) - 2 * i32::from(map.margin);
    let place_h = i32::from(map.height) - 2 * i32::from(map.margin);
    if place_w < 1 || place_h < 1 {
        return Err(invalid(
            "BV-08",
            format!("駅を置ける範囲が {place_w} × {place_h}"),
        ));
    }
    let max_stations = max_stations(spec);
    let place_cells = u32::try_from(place_w * place_h).unwrap_or(0);
    if max_stations > place_cells || max_stations > 255 {
        return Err(invalid(
            "BV-08",
            format!("max_stations = {max_stations}(置けるマス {place_cells}、上限 255)"),
        ));
    }
    let (aw, ah) = (i32::from(map.initial_area.0), i32::from(map.initial_area.1));
    if aw < 1 || ah < 1 || aw > place_w || ah > place_h {
        return Err(invalid(
            "BV-08",
            format!("initial_area = ({aw}, {ah}) が置ける範囲 {place_w} × {place_h} に収まらない"),
        ));
    }
    if company.stations_base < 2 {
        return Err(invalid(
            "BV-08",
            format!("stations_base = {}", company.stations_base),
        ));
    }

    let t = &spec.transit;
    if t.dwell_base_ms > t.dwell_max_ms || t.transfer_cost < 1 || t.waiting_cap_factor < 1 {
        return Err(invalid(
            "BV-12",
            format!(
                "dwell_base_ms = {}、dwell_max_ms = {}、transfer_cost = {}、waiting_cap_factor = {}",
                t.dwell_base_ms, t.dwell_max_ms, t.transfer_cost, t.waiting_cap_factor
            ),
        ));
    }

    for f in FEATURES {
        let n = spec.features.iter().filter(|u| u.feature == f).count();
        if n != 1 {
            return Err(invalid("BV-13", format!("{f:?} が {n} 回")));
        }
    }
    if let Some(u) = spec
        .features
        .iter()
        .find(|u| matches!(u.feature, Feature::Param(_)))
    {
        return Err(invalid(
            "BV-13",
            format!("features に {:?} がある", u.feature),
        ));
    }

    if t.dwell_base_ms < 1 {
        return Err(invalid("BV-15", "dwell_base_ms = 0".to_string()));
    }

    if map.min_spacing < 1 || map.relax_spacings.contains(&0) || map.attempts_per_stage < 1 {
        return Err(invalid(
            "BV-16",
            format!(
                "min_spacing = {}、relax_spacings = {:?}、attempts_per_stage = {}",
                map.min_spacing, map.relax_spacings, map.attempts_per_stage
            ),
        ));
    }

    let max_level_any = spec
        .params
        .iter()
        .map(|p| {
            p.intrinsic_max
                .map_or(cap_max, |m| u32::from(m).min(cap_max))
        })
        .max()
        .unwrap_or(0);
    for p in &spec.params {
        let mut prev = 0u32;
        for st in &p.cost_steps {
            let from = u32::from(st.from);
            if from <= prev || from > max_level_any || st.ratio < 1 {
                return Err(invalid(
                    "BV-18",
                    format!(
                        "{:?}: cost_steps = {:?}(max_level_any = {max_level_any})",
                        p.key, p.cost_steps
                    ),
                ));
            }
            prev = from;
        }
    }

    for p in &spec.params {
        let mut prev = 0u32;
        let geometric = matches!(p.value, ValueFormula::Geometric { .. });
        for st in &p.value_steps {
            let from = u32::from(st.from);
            if !geometric
                || p.key == ParamKey::S2Fare
                || from <= prev
                || from > max_level_any
                || st.ratio < 1
            {
                return Err(invalid(
                    "BV-21",
                    format!(
                        "{:?}: value_steps = {:?}(Geometric で S2 以外、max_level_any = {max_level_any})",
                        p.key, p.value_steps
                    ),
                ));
            }
            prev = from;
        }
    }

    let floor = &spec.economy.upgrade_floor;
    let s1_reach = spec.params[ParamKey::S1Spawn.index()]
        .intrinsic_max
        .map_or(cap_max, |m| u32::from(m).min(cap_max));
    let floor_ref = company
        .max_level
        .checked_sub(floor.behind)
        .map_or(0, |c| cap_u32(company, u32::from(c)));
    if floor.behind < 1 || s1_reach < floor_ref {
        return Err(invalid(
            "BV-22",
            format!(
                "upgrade_floor = {floor:?}、S1 の表は Lv{s1_reach} まで(基準の Lv{floor_ref} が要る)"
            ),
        ));
    }

    let tr = &spec.economy.track;
    let tiles = u32::from(map.width) * u32::from(map.height);
    let (num, den) = tr.tile_ratio;
    if tr.c1_yen < 1
        || tr.level_ratio < 1
        || den < 1
        || num < den
        || tr.free_below_level < 1
        || tr.free_below_level > company.max_level
        || tr.tile_cap > MAX_MAP_TILES
    {
        return Err(invalid(
            "BV-19",
            format!("track = {tr:?}(max_level = {})", company.max_level),
        ));
    }

    if tiles > MAX_MAP_TILES {
        return Err(invalid(
            "BV-20",
            format!(
                "マップ {} × {} が {MAX_MAP_TILES} マスを超える",
                map.width, map.height
            ),
        ));
    }
    Ok(())
}

/// 転生の検査(BV-23)。
fn check_rebirth(spec: &BalanceSpec) -> Result<(), BalanceError> {
    let r = &spec.rebirth;
    if r.points_base < 1 || r.points_per_level < 1 {
        return Err(invalid(
            "BV-23",
            format!(
                "points_base = {}、points_per_level = {}",
                r.points_base, r.points_per_level
            ),
        ));
    }
    let keys: Vec<RebirthUpgrade> = r.upgrades.iter().map(|u| u.key).collect();
    if keys != RebirthUpgrade::ALL {
        return Err(invalid("BV-23", format!("upgrades の並び = {keys:?}")));
    }
    for u in &r.upgrades {
        if u.max < 1 || u.base_cost < 1 || u.cost_ratio < 1 {
            return Err(invalid(
                "BV-23",
                format!(
                    "{:?}: max = {}、base_cost = {}、cost_ratio = {}",
                    u.key, u.max, u.base_cost, u.cost_ratio
                ),
            ));
        }
        // 最大レベルまでの倍率が u64 に収まること(コストと運賃の倍率で使う)。
        let top = u128::from(u.base_cost)
            .checked_mul((u128::from(u.cost_ratio)).pow(u32::from(u.max - 1)));
        if top.is_none_or(|v| v > u128::from(u64::MAX)) {
            return Err(invalid(
                "BV-23",
                format!("{:?}: 最大レベルのコストが u64 を超える", u.key),
            ));
        }
    }
    let fare_top = (u128::from(r.fare_boost_ratio)).checked_pow(u32::from(
        r.upgrades
            .get(RebirthUpgrade::FareBoost.index())
            .map_or(0, |u| u.max),
    ));
    if r.fare_boost_ratio < 1 || fare_top.is_none_or(|v| v > u128::from(u64::MAX)) {
        return Err(invalid(
            "BV-23",
            format!("fare_boost_ratio = {} が u64 を超える", r.fare_boost_ratio),
        ));
    }
    if r.train_boost_trains < 1 || r.line_boost_slots < 1 {
        return Err(invalid(
            "BV-23",
            format!(
                "train_boost_trains = {}、line_boost_slots = {}",
                r.train_boost_trains, r.line_boost_slots
            ),
        ));
    }
    Ok(())
}

fn max_stations(spec: &BalanceSpec) -> u32 {
    let c = &spec.company;
    u32::from(c.stations_base) + u32::from(c.max_level) * u32::from(c.stations_per_level)
}

fn overflow(what: String) -> BalanceError {
    invalid("BV-10", what)
}

fn checked_u64(v: u128, what: &str) -> Result<u64, BalanceError> {
    u64::try_from(v).map_err(|_| overflow(format!("{what} が u64 を超える")))
}

fn derive(spec: &BalanceSpec) -> Result<BalanceDerived, BalanceError> {
    let max_c = CompanyLevel(spec.company.max_level);
    let max_reach: [Level; 12] = ParamKey::ALL.map(|p| derived::limit(spec, p, max_c));
    let max_level_any = max_reach.iter().copied().max().unwrap_or(Level::ZERO);

    let mut values: [Vec<u64>; 12] = Default::default();
    for p in ParamKey::ALL {
        if p == ParamKey::S2Fare {
            continue;
        }
        let def = &spec.params[p.index()];
        let table = (0..=max_reach[p.index()].0)
            .map(|l| {
                eval_u64(def, l).ok_or_else(|| overflow(format!("{p:?} Lv{l} の値が u64 を超える")))
            })
            .collect::<Result<Vec<u64>, _>>()?;
        let u32_unit = !matches!(
            p,
            ParamKey::S1Spawn
                | ParamKey::S3StationCap
                | ParamKey::T3Boarding
                | ParamKey::T4CarCapacity
        );
        if u32_unit && table.iter().any(|&v| v > u64::from(u32::MAX)) {
            return Err(overflow(format!("{p:?} の値が u32 を超える")));
        }
        values[p.index()] = table;
    }

    let s2 = &spec.params[ParamKey::S2Fare.index()];
    let fare = (0..=max_reach[ParamKey::S2Fare.index()].0)
        .map(|l| {
            eval_money(s2, l).ok_or_else(|| overflow(format!("S2Fare Lv{l} の値が u64 を超える")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if fare.contains(&BigNum::MAX) {
        return Err(invalid(
            "BV-11",
            "S2Fare の値が BigNum::MAX に飽和した".to_string(),
        ));
    }

    let mut cost: [Vec<BigNum>; 12] = Default::default();
    for p in ParamKey::ALL {
        let def = &spec.params[p.index()];
        let table = cost_table(def, max_reach[p.index()].0);
        if table.contains(&BigNum::MAX) {
            return Err(invalid(
                "BV-11",
                format!("{p:?} のコストが BigNum::MAX に飽和した"),
            ));
        }
        cost[p.index()] = table;
    }

    let top = |p: ParamKey| u128::from(*values[p.index()].last().unwrap_or(&0));
    checked_u64(
        top(ParamKey::S1Spawn) * top(ParamKey::R3Attractiveness) / 1000 + 999,
        "発生の累積器の最大",
    )?;
    checked_u64(
        top(ParamKey::T4CarCapacity)
            .checked_mul(top(ParamKey::T5Cars) * top(ParamKey::T2Crowding))
            .map_or(u128::MAX, |v| v / 4),
        "列車の定員の最大",
    )?;
    let map = &spec.map;
    let max_track =
        u128::from(max_stations(spec).saturating_sub(1)) * u128::from(map.width.max(map.height));
    checked_u64(
        100_000
            * (1000 * max_track
                + u128::from(spec.transit.stop_allowance_s) * top(ParamKey::T1Speed)),
        "走行コストの N",
    )?;
    // 走行コストの D = v × a は、両方が u32 に収まる(上で検査済み)ので u64 を超えない。
    let t = &spec.transit;
    checked_u64(
        u128::from(t.dwell_max_ms - t.dwell_base_ms) * top(ParamKey::T3Boarding) / 1000,
        "乗車の枠",
    )?;

    let tr = &spec.economy.track;
    let track_pow_c = derived::track_pow_c(tr, spec.company.max_level);
    let track_pow_n = derived::track_pow_n(tr);
    let tiles = u64::from(spec.map.width) * u64::from(spec.map.height);
    let build_max = track_pow_c[usize::from(spec.company.max_level)]
        .saturating_mul(track_pow_n[tr.tile_cap as usize])
        .mul_small(tiles);
    if build_max == BigNum::MAX {
        return Err(invalid(
            "BV-19",
            "1 回の建設の値段の上限が BigNum::MAX に飽和した".to_string(),
        ));
    }

    let feature_unlock = FEATURES.map(|f| {
        let u = spec
            .features
            .iter()
            .find(|u| u.feature == f)
            .map_or(0, |u| u.unlock);
        CompanyLevel(u)
    });

    let value_at = |p: ParamKey, l: Level| values[p.index()][usize::from(l.0)];
    let max_lines = u32::try_from(value_at(
        ParamKey::R2LineSlots,
        max_reach[ParamKey::R2LineSlots.index()],
    ))
    .unwrap_or(u32::MAX);
    let max_trains_per_line = u32::try_from(value_at(
        ParamKey::R1MaxTrains,
        max_reach[ParamKey::R1MaxTrains.index()],
    ))
    .unwrap_or(u32::MAX);

    Ok(BalanceDerived {
        max_stations: max_stations(spec),
        max_lines,
        max_trains_per_line,
        max_level_any,
        max_reach,
        values,
        fare,
        cost,
        goal: BigNum::pow10(spec.economy.goal_pow10),
        track_pow_c,
        track_pow_n,
        feature_unlock,
    })
}

fn check_tables(spec: &BalanceSpec, d: &BalanceDerived) -> Result<(), BalanceError> {
    let value = |p: ParamKey, l: Level| d.values[p.index()][usize::from(l.0)];

    for c in 0..=spec.company.max_level {
        let c = CompanyLevel(c);
        let platform = value(
            ParamKey::S4Platform,
            derived::limit(spec, ParamKey::S4Platform, c),
        );
        let cars = value(ParamKey::T5Cars, derived::limit(spec, ParamKey::T5Cars, c));
        if platform < cars {
            return Err(invalid(
                "BV-06",
                format!("{c}: ホーム長 {platform} < 両数 {cars}"),
            ));
        }
    }

    let nodes = 2 * u64::from(d.max_stations) + u64::from(d.max_lines) * u64::from(d.max_stations);
    if nodes > MAX_ROUTE_NODES {
        return Err(invalid(
            "BV-09",
            format!("経路のノード数 {nodes} が {MAX_ROUTE_NODES} を超える"),
        ));
    }

    let top_speed = value(ParamKey::T1Speed, d.max_reach[ParamKey::T1Speed.index()]);
    if top_speed >= POS_UNITS_PER_TILE {
        return Err(invalid(
            "BV-14",
            format!("速度の最大 {top_speed} が 1 マス以上"),
        ));
    }

    let (platform0, cars0) = (
        value(ParamKey::S4Platform, Level::ZERO),
        value(ParamKey::T5Cars, Level::ZERO),
    );
    if platform0 < cars0 {
        return Err(invalid(
            "BV-17",
            format!("Lv0 のホーム長 {platform0} < 両数 {cars0}"),
        ));
    }
    Ok(())
}
