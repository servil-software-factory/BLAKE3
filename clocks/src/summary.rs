//! The one way every measurement summarises and compares its samples, in
//! BLAKE3-Servil, bench-hashes, and every probe.
//!
//! **A run's cost** for a cell is its mean: the total timed nanoseconds of
//! its samples over the total work they did, every sample counted
//! ([`mean`]). That is what a caller pays on average, and it moves
//! continuously when a cell spends more or less of its time at a slower
//! speed; a slow sample among few moves it in full, which is what the
//! caller met.
//!
//! **Comparing two programs** takes pairs of runs, one of each, back to back
//! and in alternating order, so that drift between the runs falls on both
//! sides alike. Each pair gives one ratio per cell, new mean over old. Most
//! of a measurement's uncertainty lies between runs (where the program's
//! code landed, where its threads were placed), which no figure from inside
//! one run can see; the pairs see it. A cell's change is the geometric mean
//! of its pairs' ratios ([`geometric_mean_permille`]), and every cell of a
//! comparison is judged at once ([`discoveries`]): a paired t-test on the
//! log ratios, the false discovery rate controlled over all the cells, and
//! a margin. Callers print the ratio each [`Judgment`] carries: the figure
//! judged.
//!
//! **Before changing these rules**, read bench-hashes' `METHODOLOGY.md`,
//! "Each part of the rule, and its evidence": each choice's evidence and
//! what it gives up.
//!
//! **Values** are nanoseconds per unit in fixed point, 64 integer and 64
//! fractional bits (Q64.64, as bench-hashes' `Fixed`); ratios in permille;
//! integers throughout, rounded once where a person reads them.

/// The mean of samples given as (timed ns, units of work): total ns over
/// total units, in Q64.64 ns per unit, rounded to the nearest, every
/// sample counted (Zooko, October 23, 2026, on bench-hashes #60; it
/// replaces the slowest-hundredth rule of October 19). At the budgets the
/// benchmark runs on, 6 to 24 samples a cell, setting one sample aside
/// erased a slow state that came once in ten samples (10% of the raw cost,
/// read as level: the data branch, 2026-10-22/mean-trim-controls); the
/// mean of every sample shows it, at the price of moving in full for a
/// stall, which the pairs of runs and their load readings are there to
/// catch. Requires some samples, some units, and a total under 2^64 ns.
pub fn mean(samples: impl IntoIterator<Item = (u64, u64)>) -> u128 {
    let (ns, units) = mean_totals(samples);
    ((ns << 64) + units / 2) / units
}

/// The exact nanoseconds and units [`mean`] is the ratio of: every
/// sample's, summed. Exact decimal displays use these totals; fixed-point
/// comparisons use [`mean`]. Both share this one rule and its
/// preconditions.
pub fn mean_totals(samples: impl IntoIterator<Item = (u64, u64)>) -> (u128, u128) {
    let mut count = 0usize;
    let (ns, units) = samples.into_iter().fold((0u128, 0u128), |(ns, units), (n, u)| {
        count += 1;
        (ns + u128::from(n), units + u128::from(u))
    });
    assert!(count > 0, "a mean needs samples");
    assert!(units > 0, "a mean needs some work done");
    assert!(ns < 1 << 64, "a run's samples total under 2^64 ns");
    (ns, units)
}

/// `new / old` in permille, rounded. Requires `old > 0`.
pub fn ratio_permille(new: u128, old: u128) -> u64 {
    assert!(old > 0, "a ratio needs a divisor");
    let (new, old) = (new >> 8, old >> 8); // headroom for the factor of 1000
    assert!(old > 0, "a ratio needs a divisor above 2^-56 ns per unit");
    u64::try_from((new * 1000 + old / 2) / old).expect("a ratio fits in u64")
}

/// What the pairs say of one cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Slower,
    Faster,
    Level,
}

/// A cell's judgment: its verdict; the ratio judged, the geometric mean
/// of its pairs' ratios (new over old, permille); and its rank, its place
/// among the cells judged together by the strength of the evidence that it
/// changed (its p-value), 0 for the strongest. A level cell's rank says
/// which to measure again: its ratio alone favors the noisiest cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Judgment {
    pub verdict: Verdict,
    pub ratio_permille: u64,
    pub rank: usize,
}

/// The mean of the pairs' log ratios, ln(new / old). Requires some pairs,
/// every mean positive.
fn mean_log_ratio(pairs: &[(u128, u128)]) -> f64 {
    assert!(!pairs.is_empty(), "a ratio needs pairs");
    pairs
        .iter()
        .map(|&(old, new)| {
            assert!(old > 0 && new > 0, "means are positive");
            (new as f64).ln() - (old as f64).ln()
        })
        .sum::<f64>()
        / pairs.len() as f64
}

/// The geometric mean of the pairs' ratios, new over old (each pair's means
/// as [`mean`] gives them), in permille, rounded: the change [`discoveries`]
/// judges. Requires some pairs, every mean positive.
pub fn geometric_mean_permille(pairs: &[(u128, u128)]) -> u64 {
    (mean_log_ratio(pairs).exp() * 1000.0).round() as u64
}

/// Which cells of a comparison changed, every cell judged at once: for each
/// cell, its pairs' means (old, new; fixed point, as [`mean`] gives them),
/// a paired t-test on log(new / old); the cells whose p-values pass the
/// Benjamini-Hochberg procedure at a false discovery rate of `q_permille`
/// over all the cells; and of those, the ones whose geometric mean ratio
/// lies beyond `margin_permille` slower or faster. The rest are level.
///
/// Why (bench-hashes audit/variance-on-unchanged-code.md, i7-12700K):
/// unchanged code read faster or slower in 15-27 of 700 cells under a
/// sign test per cell (5 pairs): cold calls vary in cycles with each
/// run's address layout and each binary's link layout, and 700 tests at
/// a few percent each add up. This rule found none in the same data, and
/// the code's changes when every pair ran fresh layouts. Statistics need
/// floating point here; inputs and verdicts stay exact.
///
/// Requires at least 3 pairs per cell, every cell the same count, and
/// every mean positive.
pub fn discoveries(cells: &[Vec<(u128, u128)>], q_permille: u64, margin_permille: u64) -> Vec<Judgment> {
    let pairs = cells.first().map_or(0, Vec::len);
    assert!(pairs >= 3 && cells.iter().all(|c| c.len() == pairs), "every cell has the same count of pairs, at least 3");
    // (p-value, geometric mean ratio) per cell.
    let tested: Vec<(f64, f64)> = cells
        .iter()
        .map(|c| {
            let mu = mean_log_ratio(c);
            let d: Vec<f64> = c.iter().map(|&(old, new)| (new as f64).ln() - (old as f64).ln()).collect();
            let n = d.len() as f64;
            let var = d.iter().map(|x| (x - mu) * (x - mu)).sum::<f64>() / (n - 1.0);
            let p = if var == 0.0 {
                if mu == 0.0 {
                    1.0
                } else {
                    0.0
                }
            } else {
                t_two_sided(mu / (var / n).sqrt(), n - 1.0)
            };
            (p, mu.exp())
        })
        .collect();
    // Benjamini-Hochberg: the largest rank k with p_(k) <= q k / m.
    let m = cells.len();
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by(|&a, &b| tested[a].0.total_cmp(&tested[b].0));
    let q = q_permille as f64 / 1000.0;
    let passed = (1..=m).rev().find(|&k| tested[order[k - 1]].0 <= q * k as f64 / m as f64).unwrap_or(0);
    let mut verdicts = vec![Verdict::Level; m];
    let margin = margin_permille as f64 / 1000.0;
    for &i in &order[..passed] {
        let ratio = tested[i].1;
        if ratio > 1.0 + margin {
            verdicts[i] = Verdict::Slower;
        } else if ratio < 1.0 - margin {
            verdicts[i] = Verdict::Faster;
        }
    }
    let mut rank = vec![0; m];
    for (r, &i) in order.iter().enumerate() {
        rank[i] = r;
    }
    verdicts.into_iter().zip(cells).zip(rank).map(|((verdict, c), rank)| Judgment { verdict, ratio_permille: geometric_mean_permille(c), rank }).collect()
}

/// Student's t two-sided p-value: P(|T| >= |t|) with `df` degrees of
/// freedom, the regularized incomplete beta I(df / (df + t^2); df / 2, 1/2).
fn t_two_sided(t: f64, df: f64) -> f64 {
    incomplete_beta(df / 2.0, 0.5, df / (df + t * t))
}

/// The regularized incomplete beta I_x(a, b), by its continued fraction
/// (Numerical Recipes, betai and betacf).
fn incomplete_beta(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let front = (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * beta_fraction(a, b, x) / a
    } else {
        1.0 - front * beta_fraction(b, a, 1.0 - x) / b
    }
}

fn beta_fraction(a: f64, b: f64, x: f64) -> f64 {
    let tiny = 1e-300;
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    d = 1.0 / if d.abs() < tiny { tiny } else { d };
    let mut h = d;
    for m in 1..300 {
        let m = m as f64;
        let m2 = 2.0 * m;
        for aa in [m * (b - m) * x / ((qam + m2) * (a + m2)), -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2))] {
            d = 1.0 + aa * d;
            d = 1.0 / if d.abs() < tiny { tiny } else { d };
            c = 1.0 + aa / c;
            c = if c.abs() < tiny { tiny } else { c };
            h *= d * c;
        }
        if (d * c - 1.0).abs() < 1e-15 {
            break;
        }
    }
    h
}

/// ln Gamma(x) for x >= 1/2 (Lanczos, g = 7, 9 terms).
fn ln_gamma(x: f64) -> f64 {
    const G: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    // The callers' arguments are df / 2 (one or more: three pairs at
    // least), 1/2 and their sum; the reflection for x below 1/2 is not needed.
    debug_assert!(x >= 0.5, "ln_gamma below 1/2 would need the reflection formula");
    let x = x - 1.0;
    let mut a = G[0];
    let t = x + 7.5;
    for (i, g) in G.iter().enumerate().skip(1) {
        a += g / (x + i as f64);
    }
    0.5 * (2.0 * core::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Student's t two-sided p-values at tabled points (Abramowitz and
    /// Stegun's table 26.10, and the closed forms at one degree of freedom,
    /// p = 1 - 2 arctan(t) / pi), within 1e-4: the incomplete beta's
    /// continued fraction and its front factor, both ways round the
    /// symmetry point.
    #[test]
    fn t_two_sided_matches_the_tables() {
        let close = |t: f64, df: f64, p: f64| {
            let got = t_two_sided(t, df);
            assert!((got - p).abs() < 1e-4, "t {t} with {df} degrees: p {got}, the table says {p}");
        };
        close(1.0, 1.0, 0.5);
        close(12.7062, 1.0, 0.05);
        close(63.657, 1.0, 0.01);
        close(2.0, 1.0, 1.0 - 2.0 * 2.0f64.atan() / std::f64::consts::PI);
        close(3.1824, 3.0, 0.05);
        close(2.3534, 3.0, 0.10);
        close(2.2281, 10.0, 0.05);
        close(3.1693, 10.0, 0.01);
        close(0.7, 10.0, 0.4999);
        close(2.0423, 30.0, 0.05);
        close(1.96, 1000.0, 0.0503);
        // Zero: every value as likely; huge: certain. The extremes take
        // the incomplete beta's two branches at their far ends (x near 1
        // and near 0), where the other branch's continued fraction would
        // converge too slowly for 300 terms.
        close(0.0, 5.0, 1.0);
        assert!(t_two_sided(1000.0, 5.0) < 1e-12, "t 1000: certain, {}", t_two_sided(1000.0, 5.0));
        let nearly_one = t_two_sided(0.001, 2.0);
        assert!(
            (nearly_one - (1.0 - 0.001 * std::f64::consts::FRAC_1_SQRT_2)).abs() < 1e-6,
            "t 0.001 at 2 degrees: p = 1 - t / sqrt(2) to first order, {nearly_one}"
        );
    }

    /// Benjamini-Hochberg at q: the largest k with p_(k) <= q k / m passes
    /// the k smallest p-values. Three cells, p 0, about 0.03 and about 0.4,
    /// at q = 0.05: k = 2 passes (0.03 <= 0.033), k = 3 does not
    /// (0.4 > 0.05), so the certain and the 0.03 cells are judged and the
    /// noisy +15% cell stays level however far its mean lies.
    #[test]
    fn discoveries_pass_the_largest_k_under_the_benjamini_hochberg_line() {
        let e = |x: f64| (1_000_000.0 * x.exp()) as u128;
        let cells = vec![
            vec![(1_000_000, 1_100_000); 4],
            vec![(1_000_000, e(0.1)), (1_000_000, e(0.2)), (1_000_000, e(0.3)), (1_000_000, e(0.4))],
            vec![(1_000_000, e(0.6)), (1_000_000, e(-0.2)), (1_000_000, e(0.3)), (1_000_000, e(-0.1))],
        ];
        let verdicts: Vec<Verdict> = discoveries(&cells, 50, 30).into_iter().map(|j| j.verdict).collect();
        assert_eq!(verdicts, [Verdict::Slower, Verdict::Slower, Verdict::Level]);
        // At q = 0.02 the 0.03 cell fails k = 2 (0.03 > 0.0133) and the
        // certain one alone passes.
        let verdicts: Vec<Verdict> = discoveries(&cells, 20, 30).into_iter().map(|j| j.verdict).collect();
        assert_eq!(verdicts, [Verdict::Slower, Verdict::Level, Verdict::Level]);
    }

    /// Pairs that all moved by the same ratio are certain (p 0, whatever
    /// the direction); pairs that all read level are no evidence (p 1); the
    /// variance is the sample's, over n - 1: four log ratios 0.1, 0.2, 0.3,
    /// 0.4 give t = 0.25 / sqrt(0.016667 / 4) = 3.873 at 3 degrees, p 0.0305.
    #[test]
    fn a_cells_evidence_is_its_pairs_t_statistic() {
        let e = |x: f64| (1_000_000.0 * x.exp()) as u128;
        let cells = vec![
            vec![(1_000_000, 1_100_000); 4],
            vec![(1_000_000, 1_000_000); 4],
            vec![(1_000_000, e(0.1)), (1_000_000, e(0.2)), (1_000_000, e(0.3)), (1_000_000, e(0.4))],
            vec![(1_000_000, 900_000); 4],
        ];
        // q = 0.04: the cell at p 0.0305 passes alone when it is the only
        // one near the threshold (Benjamini-Hochberg at k = 3 of 4 needs
        // p <= 0.03), so judge it on its own.
        let alone = discoveries(&cells[2..3], 31, 30);
        assert_eq!(alone[0].verdict, Verdict::Slower, "p 0.0305 <= 0.031");
        let alone = discoveries(&cells[2..3], 30, 30);
        assert_eq!(alone[0].verdict, Verdict::Level, "p 0.0305 > 0.030");
        let all = discoveries(&cells, 50, 30);
        assert_eq!(all[0].verdict, Verdict::Slower, "identical +10% pairs: certain");
        assert_eq!(all[1].verdict, Verdict::Level, "identical level pairs: no evidence");
        assert_eq!(all[3].verdict, Verdict::Faster, "identical -10% pairs: certain");
        assert!(all[0].rank < all[2].rank && all[2].rank < all[1].rank, "ranked by evidence: certain, then p 0.03, then p 1");
    }

    #[test]
    fn exact_mean_totals_are_every_sample_summed() {
        let mut samples = vec![(1000, 1); 99];
        samples.push((1_000_000, 1));
        assert_eq!(mean_totals(samples.clone()), (99_000 + 1_000_000, 100));
        assert_eq!(mean(samples), 10_990 << 64);
        assert_eq!(mean_totals([(200, 100), (1000, 1)]), (1200, 101));
    }

    #[test]
    fn a_mean_is_total_time_over_total_work() {
        // 99 samples of 5 ns over 2 units and one of 7 ns over 2 units:
        // 502 ns over 200 units, 2.51 ns a unit.
        let mut samples = vec![(5, 2); 99];
        samples.push((7, 2));
        assert_eq!(mean(samples), ((502u128 << 64) + 100) / 200);
        assert_eq!(ratio_permille(mean([(110, 100)]), mean([(100, 100)])), 1100);
    }

    /// Every sample counts, the slowest included: the reason is a slow
    /// state that comes once in ten samples, which a rule setting one
    /// sample aside erased entirely at six samples (bench-hashes' data
    /// branch, 2026-10-22/mean-trim-controls: ten phases, each read as
    /// level, 10% of the raw cost gone). This test is that fixture in
    /// small: a rule that drops the slowest sample reads 1000 here and
    /// fails it.
    #[test]
    fn a_slow_sample_among_few_counts_in_full() {
        // Six samples, one of them twice as slow: 7000 ns over 6 units.
        let samples = [(1000, 1), (1000, 1), (2000, 1), (1000, 1), (1000, 1), (1000, 1)];
        assert_eq!(mean_totals(samples), (7000, 6));
        assert_eq!(mean(samples), ((7000u128 << 64) + 3) / 6);
        // Two samples: the slow one is half the figure.
        assert_eq!(mean([(100, 1), (1000, 1)]), 550 << 64);
        // Slowness is per unit, and so is the mean: 200 ns over 100 units
        // weighs as 100 units.
        let mut samples = vec![(100, 1); 98];
        samples.extend([(200, 100), (1000, 1)]);
        assert_eq!(mean(samples), ((((98 * 100 + 200 + 1000) as u128) << 64) + 99) / 199);
    }

    /// The geometric mean of the ratios: 1.21 and 1 give 1.1; 1.1 and 1/1.1
    /// give 1; a single pair gives its own ratio.
    #[test]
    fn the_change_is_the_geometric_mean_of_the_ratios() {
        let pair = |old: u128, new: u128| (old << 64, new << 64);
        assert_eq!(geometric_mean_permille(&[pair(100, 121), pair(100, 100)]), 1100);
        assert_eq!(geometric_mean_permille(&[pair(100, 110), pair(110, 100)]), 1000);
        assert_eq!(geometric_mean_permille(&[pair(200, 150)]), 750);
    }

    /// Student's t at its tables' points: t = 2.776 with 4 degrees of
    /// freedom is p = 0.05 two-sided; 2.228 with 10, 0.05; 3.169 with 10,
    /// 0.01; t = 0, p = 1.
    #[test]
    fn t_p_values_match_the_tables() {
        for (t, df, p) in [(2.776, 4.0, 0.05), (2.228, 10.0, 0.05), (3.169, 10.0, 0.01), (0.0, 4.0, 1.0)] {
            assert!((t_two_sided(t, df) - p).abs() < 2e-4, "t {t} df {df}: {}", t_two_sided(t, df));
        }
    }

    /// Three cells of five pairs: one 10% slower in every pair, one whose
    /// pairs scatter 3% each way, one 1% faster in every pair (its
    /// p-value passes, but it lies within the 3% margin given). The slower cell is
    /// still found among 1000 unchanged ones; at q = 0 nothing is.
    #[test]
    fn discoveries_judge_every_cell_at_once() {
        let pairs = |ratios: &[u128]| ratios.iter().map(|&r| (1000u128 << 63, r << 63)).collect::<Vec<_>>();
        let cells = vec![pairs(&[1100, 1102, 1098, 1101, 1099]), pairs(&[1030, 970, 1025, 975, 1000]), pairs(&[990, 991, 989, 990, 990])];
        let judged = discoveries(&cells, 50, 30);
        assert_eq!(judged.iter().map(|j| j.verdict).collect::<Vec<_>>(), [Verdict::Slower, Verdict::Level, Verdict::Level]);
        // Each judgment carries the ratio judged: the pairs' geometric mean.
        assert_eq!(judged.iter().map(|j| j.ratio_permille).collect::<Vec<_>>(), [1100, 1000, 990]);
        let mut many = vec![pairs(&[1030, 970, 1025, 975, 1000]); 1000];
        many.push(pairs(&[1100, 1102, 1098, 1101, 1099]));
        assert_eq!(discoveries(&many, 50, 30).iter().filter(|j| j.verdict != Verdict::Level).count(), 1);
        assert!(discoveries(&many, 0, 30).iter().all(|j| j.verdict == Verdict::Level));
    }

    /// Ranks follow the evidence, not the ratio: a cell 4% slower in every
    /// pair outranks one whose pairs scatter to a 10% mean.
    #[test]
    fn discoveries_rank_by_evidence() {
        let pairs = |ratios: &[u128]| ratios.iter().map(|&r| (1000u128 << 63, r << 63)).collect::<Vec<_>>();
        let cells = vec![pairs(&[1300, 900, 1250, 950, 1100]), pairs(&[1040, 1041, 1039, 1040, 1040]), pairs(&[1030, 970, 1025, 975, 1000])];
        let judged = discoveries(&cells, 50, 30);
        assert!(judged[0].ratio_permille > judged[1].ratio_permille);
        assert_eq!(judged.iter().map(|j| j.rank).collect::<Vec<_>>(), [1, 0, 2]);
    }
}
