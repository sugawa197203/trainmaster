use tm_numeric::BigNum;

use crate::RULES_VERSION;
use crate::balance::{BALANCE_FORMAT_VERSION, BalanceTable, ValueFormula};
use crate::param::Feature;
use crate::time::TICKS_PER_SEC;
use crate::units::Money;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 固定幅・固定順序の FNV-1a 64(10 TD-05)。整数はリトルエンディアン。`std::hash` は使わない。
#[derive(Clone, Debug)]
pub struct StateHasher {
    state: u64,
}

impl Default for StateHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl StateHasher {
    /// オフセット基底から始める。
    pub fn new() -> Self {
        StateHasher { state: FNV_OFFSET }
    }

    /// バイト列をそのまま書く(長さは書かない)。識別子 `b"TMSH"` などに使う。
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state ^= u64::from(b);
            self.state = self.state.wrapping_mul(FNV_PRIME);
        }
    }

    /// 1 バイト。
    pub fn write_u8(&mut self, v: u8) {
        self.write_bytes(&[v]);
    }

    /// 2 バイト。
    pub fn write_u16(&mut self, v: u16) {
        self.write_bytes(&v.to_le_bytes());
    }

    /// 4 バイト。
    pub fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }

    /// 8 バイト。
    pub fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }

    /// 16 バイト。
    pub fn write_u128(&mut self, v: u128) {
        self.write_bytes(&v.to_le_bytes());
    }

    /// 4 バイト(2 の補数)。
    pub fn write_i32(&mut self, v: i32) {
        self.write_bytes(&v.to_le_bytes());
    }

    /// `u8` の 0 / 1。
    pub fn write_bool(&mut self, v: bool) {
        self.write_u8(u8::from(v));
    }

    /// `to_bits()`(−0.0 は 0.0 に正規化済み)。
    pub fn write_money(&mut self, v: Money) {
        let bits = v.to_bits();
        debug_assert!(
            BigNum::from_bits_checked(bits).is_some(),
            "NaN・無限大・負のお金"
        );
        self.write_u64(bits);
    }

    /// 要素数を `u64` で書く。
    pub fn write_len(&mut self, n: usize) {
        self.write_u64(n as u64);
    }

    /// いまのハッシュ値。
    pub fn finish(&self) -> u64 {
        self.state
    }
}

impl BalanceTable {
    /// 識別子・版・`TICKS_PER_SEC` と `BalanceSpec` の全フィールドを宣言の順に書いたハッシュ(10 §9.3)。
    /// `Option` と enum は判別子(宣言の順の番号)を `u8` で書いてから中身を書く。
    pub fn config_hash(&self) -> u64 {
        let s = self.spec();
        let mut h = StateHasher::new();
        h.write_bytes(b"TMCF");
        h.write_u32(BALANCE_FORMAT_VERSION);
        h.write_u32(RULES_VERSION);
        h.write_u32(TICKS_PER_SEC);

        h.write_u32(s.format_version);

        let m = &s.map;
        h.write_u8(m.width);
        h.write_u8(m.height);
        h.write_u8(m.margin);
        h.write_u8(m.initial_area.0);
        h.write_u8(m.initial_area.1);
        h.write_u8(m.min_spacing);
        h.write_u8(m.max_nearest);
        h.write_len(m.relax_spacings.len());
        for &sp in &m.relax_spacings {
            h.write_u8(sp);
        }
        h.write_u32(m.attempts_per_stage);

        let c = &s.company;
        h.write_u8(c.max_level);
        h.write_u8(c.cap_base);
        h.write_u8(c.cap_step);
        h.write_u32(c.gauge_num);
        h.write_u32(c.gauge_den);
        h.write_u8(c.stations_base);
        h.write_u8(c.stations_per_level);

        h.write_len(s.params.len());
        for p in &s.params {
            h.write_u8(p.key as u8);
            h.write_u64(p.base_yen);
            h.write_u32(p.cost_ratio);
            h.write_len(p.cost_steps.len());
            for st in &p.cost_steps {
                h.write_u8(st.from);
                h.write_u32(st.ratio);
            }
            match p.intrinsic_max {
                None => h.write_u8(0),
                Some(v) => {
                    h.write_u8(1);
                    h.write_u8(v);
                }
            }
            h.write_u8(p.unlock);
            match p.value {
                ValueFormula::Linear { v0, step } => {
                    h.write_u8(0);
                    h.write_u64(v0);
                    h.write_u64(step);
                }
                ValueFormula::Geometric { v0, ratio } => {
                    h.write_u8(1);
                    h.write_u64(v0);
                    h.write_u64(ratio);
                }
            }
            h.write_len(p.value_steps.len());
            for st in &p.value_steps {
                h.write_u8(st.from);
                h.write_u32(st.ratio);
            }
        }

        h.write_len(s.features.len());
        for f in &s.features {
            write_feature(&mut h, f.feature);
            h.write_u8(f.unlock);
        }

        let t = &s.transit;
        h.write_u32(t.dwell_base_ms);
        h.write_u32(t.dwell_max_ms);
        h.write_u32(t.stop_allowance_s);
        h.write_u64(t.transfer_cost);
        h.write_u32(t.waiting_cap_factor);
        h.write_u8(t.spawn_level_margin);

        h.write_u64(s.economy.initial_money_yen);
        h.write_u32(s.economy.goal_pow10);
        let tr = &s.economy.track;
        h.write_u64(tr.c1_yen);
        h.write_u32(tr.level_ratio);
        h.write_u32(tr.tile_ratio.0);
        h.write_u32(tr.tile_ratio.1);
        h.write_u32(tr.tile_cap);
        h.write_u8(tr.free_below_level);
        h.write_u8(s.economy.upgrade_floor.behind);
        h.write_u8(s.economy.upgrade_floor.from_level);
        h.finish()
    }
}

fn write_feature(h: &mut StateHasher, f: Feature) {
    match f {
        Feature::Param(p) => {
            h.write_u8(0);
            h.write_u8(p as u8);
        }
        Feature::LineExtend => h.write_u8(1),
        Feature::BulkBuy => h.write_u8(2),
        Feature::Transfer => h.write_u8(3),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::balance::BUILTIN_BALANCE_RON;

    #[test]
    fn fnv1a_reference_vectors() {
        assert_eq!(StateHasher::new().finish(), 0xcbf2_9ce4_8422_2325);
        let mut h = StateHasher::new();
        h.write_bytes(b"a");
        assert_eq!(h.finish(), 0xaf63_dc4c_8601_ec8c);
        let mut h = StateHasher::new();
        h.write_bytes(b"foobar");
        assert_eq!(h.finish(), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn fixed_width_little_endian() {
        let mut a = StateHasher::new();
        a.write_u32(0x0403_0201);
        a.write_i32(-1);
        a.write_bool(true);
        a.write_len(2);
        a.write_u16(0x0605);
        a.write_u128(7);
        let mut b = StateHasher::new();
        b.write_bytes(&[
            1, 2, 3, 4, 0xff, 0xff, 0xff, 0xff, 1, 2, 0, 0, 0, 0, 0, 0, 0, 5, 6,
        ]);
        b.write_bytes(&[7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(a.finish(), b.finish());
    }

    #[test]
    fn money_negative_zero_is_normalized() {
        let zero = BigNum::from_u64(5)
            .checked_sub(BigNum::from_u64(5))
            .unwrap();
        let mut a = StateHasher::new();
        a.write_money(zero);
        let mut b = StateHasher::new();
        b.write_u64(0);
        assert_eq!(a.finish(), b.finish());
    }

    /// ゴールデン(10 §9.3)。`balance.ron` か書く順を変えたら更新する(16 §2.7)。
    #[test]
    fn config_hash_golden() {
        assert_eq!(BalanceTable::builtin().config_hash(), CONFIG_HASH_GOLDEN);
    }

    const CONFIG_HASH_GOLDEN: u64 = 0x9297_b64e_31aa_95ca;

    #[test]
    fn config_hash_ignores_formatting_but_not_values() {
        let base = BalanceTable::builtin().config_hash();
        let spaced = BUILTIN_BALANCE_RON
            .replace(", ", ",   ")
            .replace("// ", "// x ");
        assert_eq!(
            BalanceTable::from_ron_str(&spaced).unwrap().config_hash(),
            base
        );
        let changed = BUILTIN_BALANCE_RON.replace("dwell_max_ms: 10000", "dwell_max_ms: 9999");
        assert_ne!(
            BalanceTable::from_ron_str(&changed).unwrap().config_hash(),
            base
        );
    }
}
