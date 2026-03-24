use anyhow::Result;
use regex::Regex;
use rust_decimal::Decimal;
use std::collections::HashMap;

/// Provides a list of trading pairs.
pub trait PairListProvider: Send + Sync {
    fn name(&self) -> &str;
    fn pairs(&self) -> Result<Vec<String>>;
}

/// Filters pairs from a list.
pub trait PairFilter: Send + Sync {
    fn name(&self) -> &str;
    fn filter(&self, pairs: &[String]) -> Vec<String>;
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

/// Config-defined pair list with optional regex wildcard support.
pub struct StaticPairList {
    pairs: Vec<String>,
    patterns: Vec<String>,
}

impl StaticPairList {
    pub fn new(pairs: Vec<String>) -> Self {
        Self {
            pairs,
            patterns: Vec::new(),
        }
    }

    pub fn with_patterns(patterns: Vec<String>) -> Self {
        Self {
            pairs: Vec::new(),
            patterns,
        }
    }
}

impl PairListProvider for StaticPairList {
    fn name(&self) -> &str {
        "StaticPairList"
    }

    fn pairs(&self) -> Result<Vec<String>> {
        Ok(self.pairs.clone())
    }
}

impl PairFilter for StaticPairList {
    fn name(&self) -> &str {
        "StaticPairList"
    }

    fn filter(&self, pairs: &[String]) -> Vec<String> {
        if self.patterns.is_empty() {
            return pairs.to_vec();
        }

        let regexes: Vec<Regex> = self
            .patterns
            .iter()
            .filter_map(|p| Regex::new(p).ok())
            .collect();

        pairs
            .iter()
            .filter(|pair| regexes.iter().any(|re| re.is_match(pair)))
            .cloned()
            .collect()
    }
}

/// Sort/filter by 24h volume. Takes pre-loaded volume data.
pub struct VolumePairList {
    volumes: HashMap<String, Decimal>,
    min_volume: Decimal,
    max_pairs: usize,
}

impl VolumePairList {
    pub fn new(volumes: HashMap<String, Decimal>, min_volume: Decimal, max_pairs: usize) -> Self {
        Self {
            volumes,
            min_volume,
            max_pairs,
        }
    }
}

impl PairListProvider for VolumePairList {
    fn name(&self) -> &str {
        "VolumePairList"
    }

    fn pairs(&self) -> Result<Vec<String>> {
        let mut filtered: Vec<_> = self
            .volumes
            .iter()
            .filter(|(_, vol)| **vol >= self.min_volume)
            .collect();

        // Sort descending by volume
        filtered.sort_by(|a, b| b.1.cmp(a.1));

        let result: Vec<String> = filtered
            .into_iter()
            .take(self.max_pairs)
            .map(|(pair, _)| pair.clone())
            .collect();

        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

/// Filter by min/max price.
pub struct PriceFilter {
    prices: HashMap<String, Decimal>,
    min_price: Option<Decimal>,
    max_price: Option<Decimal>,
}

impl PriceFilter {
    pub fn new(
        prices: HashMap<String, Decimal>,
        min_price: Option<Decimal>,
        max_price: Option<Decimal>,
    ) -> Self {
        Self {
            prices,
            min_price,
            max_price,
        }
    }
}

impl PairFilter for PriceFilter {
    fn name(&self) -> &str {
        "PriceFilter"
    }

    fn filter(&self, pairs: &[String]) -> Vec<String> {
        pairs
            .iter()
            .filter(|pair| {
                let Some(price) = self.prices.get(*pair) else {
                    return false;
                };
                if let Some(min) = &self.min_price {
                    if price < min {
                        return false;
                    }
                }
                if let Some(max) = &self.max_price {
                    if price > max {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect()
    }
}

/// Filter by bid-ask spread percentage.
pub struct SpreadFilter {
    spreads: HashMap<String, Decimal>,
    max_spread_pct: Decimal,
}

impl SpreadFilter {
    pub fn new(spreads: HashMap<String, Decimal>, max_spread_pct: Decimal) -> Self {
        Self {
            spreads,
            max_spread_pct,
        }
    }
}

impl PairFilter for SpreadFilter {
    fn name(&self) -> &str {
        "SpreadFilter"
    }

    fn filter(&self, pairs: &[String]) -> Vec<String> {
        pairs
            .iter()
            .filter(|pair| {
                let Some(spread) = self.spreads.get(*pair) else {
                    return false;
                };
                *spread <= self.max_spread_pct
            })
            .cloned()
            .collect()
    }
}

/// Filter by price volatility (standard deviation).
pub struct VolatilityFilter {
    volatilities: HashMap<String, Decimal>,
    min_volatility: Option<Decimal>,
    max_volatility: Option<Decimal>,
}

impl VolatilityFilter {
    pub fn new(
        volatilities: HashMap<String, Decimal>,
        min_volatility: Option<Decimal>,
        max_volatility: Option<Decimal>,
    ) -> Self {
        Self {
            volatilities,
            min_volatility,
            max_volatility,
        }
    }
}

impl PairFilter for VolatilityFilter {
    fn name(&self) -> &str {
        "VolatilityFilter"
    }

    fn filter(&self, pairs: &[String]) -> Vec<String> {
        pairs
            .iter()
            .filter(|pair| {
                let Some(vol) = self.volatilities.get(*pair) else {
                    return false;
                };
                if let Some(min) = &self.min_volatility {
                    if vol < min {
                        return false;
                    }
                }
                if let Some(max) = &self.max_volatility {
                    if vol > max {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect()
    }
}

/// Composable pipeline of filters applied sequentially.
pub struct FilterChain {
    filters: Vec<Box<dyn PairFilter>>,
}

impl Default for FilterChain {
    fn default() -> Self {
        Self::new()
    }
}

impl FilterChain {
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
        }
    }

    pub fn add(&mut self, filter: Box<dyn PairFilter>) {
        self.filters.push(filter);
    }

    pub fn apply(&self, pairs: &[String]) -> Vec<String> {
        let mut result = pairs.to_vec();
        for filter in &self.filters {
            result = filter.filter(&result);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn static_pairlist_returns_configured_pairs() {
        let pairs = vec!["BTCUSDT".into(), "ETHUSDT".into()];
        let provider = StaticPairList::new(pairs.clone());
        assert_eq!(provider.pairs().unwrap(), pairs);
    }

    #[test]
    fn static_pairlist_with_regex_patterns_matches_correctly() {
        let provider = StaticPairList::with_patterns(vec![".*USDT$".into()]);
        let input = vec![
            "BTCUSDT".into(),
            "ETHUSDT".into(),
            "BTCBUSD".into(),
            "SOLUSDT".into(),
        ];
        let result = provider.filter(&input);
        assert_eq!(result, vec!["BTCUSDT", "ETHUSDT", "SOLUSDT"]);
    }

    #[test]
    fn volume_pairlist_sorts_by_volume_and_limits_count() {
        let mut volumes = HashMap::new();
        volumes.insert("BTCUSDT".into(), dec("1000000"));
        volumes.insert("ETHUSDT".into(), dec("500000"));
        volumes.insert("SOLUSDT".into(), dec("800000"));
        volumes.insert("DOGEUSDT".into(), dec("200000"));

        let provider = VolumePairList::new(volumes, dec("0"), 2);
        let result = provider.pairs().unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "BTCUSDT");
        assert_eq!(result[1], "SOLUSDT");
    }

    #[test]
    fn volume_pairlist_filters_below_minimum_volume() {
        let mut volumes = HashMap::new();
        volumes.insert("BTCUSDT".into(), dec("1000000"));
        volumes.insert("ETHUSDT".into(), dec("500000"));
        volumes.insert("DOGEUSDT".into(), dec("100"));

        let provider = VolumePairList::new(volumes, dec("100000"), 10);
        let result = provider.pairs().unwrap();
        assert_eq!(result.len(), 2);
        assert!(!result.contains(&"DOGEUSDT".to_string()));
    }

    #[test]
    fn price_filter_excludes_outside_range() {
        let mut prices = HashMap::new();
        prices.insert("BTCUSDT".into(), dec("60000"));
        prices.insert("ETHUSDT".into(), dec("3000"));
        prices.insert("SHIBUSDT".into(), dec("0.00001"));

        let filter = PriceFilter::new(prices, Some(dec("1")), Some(dec("50000")));
        let input: Vec<String> = vec!["BTCUSDT".into(), "ETHUSDT".into(), "SHIBUSDT".into()];
        let result = filter.filter(&input);
        assert_eq!(result, vec!["ETHUSDT"]);
    }

    #[test]
    fn spread_filter_excludes_high_spread_pairs() {
        let mut spreads = HashMap::new();
        spreads.insert("BTCUSDT".into(), dec("0.01"));
        spreads.insert("ETHUSDT".into(), dec("0.05"));
        spreads.insert("LOWCAP".into(), dec("2.5"));

        let filter = SpreadFilter::new(spreads, dec("0.1"));
        let input: Vec<String> = vec!["BTCUSDT".into(), "ETHUSDT".into(), "LOWCAP".into()];
        let result = filter.filter(&input);
        assert_eq!(result, vec!["BTCUSDT", "ETHUSDT"]);
    }

    #[test]
    fn volatility_filter_excludes_outside_range() {
        let mut volatilities = HashMap::new();
        volatilities.insert("BTCUSDT".into(), dec("2.5"));
        volatilities.insert("ETHUSDT".into(), dec("5.0"));
        volatilities.insert("STABLEUSDT".into(), dec("0.1"));

        let filter = VolatilityFilter::new(volatilities, Some(dec("1.0")), Some(dec("4.0")));
        let input: Vec<String> = vec!["BTCUSDT".into(), "ETHUSDT".into(), "STABLEUSDT".into()];
        let result = filter.filter(&input);
        assert_eq!(result, vec!["BTCUSDT"]);
    }

    #[test]
    fn filter_chain_applies_filters_sequentially() {
        let mut prices = HashMap::new();
        prices.insert("BTCUSDT".into(), dec("60000"));
        prices.insert("ETHUSDT".into(), dec("3000"));
        prices.insert("SHIBUSDT".into(), dec("0.00001"));

        let mut spreads = HashMap::new();
        spreads.insert("BTCUSDT".into(), dec("0.01"));
        spreads.insert("ETHUSDT".into(), dec("5.0"));

        let mut chain = FilterChain::new();
        chain.add(Box::new(PriceFilter::new(prices, Some(dec("1")), None)));
        chain.add(Box::new(SpreadFilter::new(spreads, dec("0.1"))));

        let input: Vec<String> = vec!["BTCUSDT".into(), "ETHUSDT".into(), "SHIBUSDT".into()];
        let result = chain.apply(&input);
        // PriceFilter removes SHIBUSDT (too cheap), SpreadFilter removes ETHUSDT (high spread)
        assert_eq!(result, vec!["BTCUSDT"]);
    }

    #[test]
    fn filter_chain_with_no_filters_passes_all() {
        let chain = FilterChain::new();
        let input: Vec<String> = vec!["BTCUSDT".into(), "ETHUSDT".into()];
        let result = chain.apply(&input);
        assert_eq!(result, input);
    }

    #[test]
    fn empty_pair_list_returns_empty() {
        let provider = StaticPairList::new(vec![]);
        assert!(provider.pairs().unwrap().is_empty());

        let volume_provider = VolumePairList::new(HashMap::new(), dec("0"), 10);
        assert!(volume_provider.pairs().unwrap().is_empty());

        let chain = FilterChain::new();
        let empty: Vec<String> = vec![];
        assert!(chain.apply(&empty).is_empty());
    }
}
