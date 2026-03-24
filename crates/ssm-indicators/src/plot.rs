use rust_decimal::prelude::ToPrimitive;
use serde::Serialize;
use ssm_core::Candle;

/// Data point for chart rendering.
#[derive(Debug, Clone, Serialize)]
pub struct ChartData {
    pub candles: Vec<CandlePoint>,
    pub indicators: Vec<IndicatorSeries>,
    pub signals: Vec<SignalMarker>,
    pub equity_curve: Vec<EquityPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CandlePoint {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndicatorSeries {
    pub name: String,
    pub chart_type: ChartType,
    pub data: Vec<DataPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub enum ChartType {
    Line,
    Histogram,
    Area,
}

#[derive(Debug, Clone, Serialize)]
pub struct DataPoint {
    pub time: i64,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignalMarker {
    pub time: i64,
    pub price: f64,
    pub action: String,
    pub profit: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EquityPoint {
    pub time: i64,
    pub equity: f64,
    pub drawdown_pct: f64,
}

impl ChartData {
    pub fn from_candles(candles: &[Candle]) -> Self {
        let candle_points = candles
            .iter()
            .map(|c| CandlePoint {
                time: c.open_time,
                open: c.open.to_f64().unwrap_or(0.0),
                high: c.high.to_f64().unwrap_or(0.0),
                low: c.low.to_f64().unwrap_or(0.0),
                close: c.close.to_f64().unwrap_or(0.0),
                volume: c.volume.to_f64().unwrap_or(0.0),
            })
            .collect();

        Self {
            candles: candle_points,
            indicators: Vec::new(),
            signals: Vec::new(),
            equity_curve: Vec::new(),
        }
    }

    pub fn add_indicator(&mut self, name: &str, chart_type: ChartType, values: &[(i64, f64)]) {
        self.indicators.push(IndicatorSeries {
            name: name.to_string(),
            chart_type,
            data: values
                .iter()
                .map(|(t, v)| DataPoint {
                    time: *t,
                    value: *v,
                })
                .collect(),
        });
    }

    pub fn add_signal(&mut self, time: i64, price: f64, action: &str, profit: Option<f64>) {
        self.signals.push(SignalMarker {
            time,
            price,
            action: action.to_string(),
            profit,
        });
    }

    pub fn add_equity(&mut self, equity_curve: Vec<(i64, f64, f64)>) {
        self.equity_curve = equity_curve
            .into_iter()
            .map(|(time, equity, drawdown_pct)| EquityPoint {
                time,
                equity,
                drawdown_pct,
            })
            .collect();
    }

    /// Export chart data as JSON for external rendering.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Generate a self-contained HTML file with Plotly.js charts.
    pub fn to_html(&self) -> String {
        let json_data = serde_json::to_string(self).unwrap_or_default();

        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>trade-ssm Chart</title>
<script src="https://cdn.plot.ly/plotly-2.27.0.min.js"></script>
<style>
  body {{ margin: 0; padding: 16px; background: #1a1a2e; color: #eee; font-family: sans-serif; }}
  .chart {{ margin-bottom: 16px; }}
</style>
</head>
<body>
<h2>trade-ssm Backtest Chart</h2>
<div id="candlestick" class="chart"></div>
<div id="volume" class="chart"></div>
<div id="equity" class="chart"></div>
<script>
(function() {{
  var data = {json_data};
  var times = data.candles.map(function(c) {{ return new Date(c.time); }});
  var opens = data.candles.map(function(c) {{ return c.open; }});
  var highs = data.candles.map(function(c) {{ return c.high; }});
  var lows  = data.candles.map(function(c) {{ return c.low; }});
  var closes = data.candles.map(function(c) {{ return c.close; }});
  var volumes = data.candles.map(function(c) {{ return c.volume; }});

  // Candlestick trace
  var candleTrace = {{
    x: times, open: opens, high: highs, low: lows, close: closes,
    type: 'candlestick',
    name: 'OHLC',
    increasing: {{ line: {{ color: '#26a69a' }} }},
    decreasing: {{ line: {{ color: '#ef5350' }} }}
  }};

  var traces = [candleTrace];

  // Indicator overlays
  data.indicators.forEach(function(ind) {{
    var indTimes = ind.data.map(function(d) {{ return new Date(d.time); }});
    var indVals  = ind.data.map(function(d) {{ return d.value; }});
    var traceType = 'scatter';
    var fill = 'none';
    if (ind.chart_type === 'Area') {{ fill = 'tozeroy'; }}
    if (ind.chart_type === 'Histogram') {{ traceType = 'bar'; }}
    traces.push({{
      x: indTimes, y: indVals,
      type: traceType, mode: 'lines', name: ind.name, fill: fill
    }});
  }});

  // Signal markers — entries
  var entrySignals = data.signals.filter(function(s) {{
    return s.action.indexOf('entry') !== -1 || s.action.indexOf('enter') !== -1;
  }});
  if (entrySignals.length > 0) {{
    traces.push({{
      x: entrySignals.map(function(s) {{ return new Date(s.time); }}),
      y: entrySignals.map(function(s) {{ return s.price; }}),
      text: entrySignals.map(function(s) {{
        return s.action + (s.profit !== null ? ' P/L: ' + s.profit.toFixed(2) : '');
      }}),
      mode: 'markers', type: 'scatter', name: 'Entry',
      marker: {{ symbol: 'triangle-up', size: 12, color: '#26a69a' }}
    }});
  }}

  var exitSignals = data.signals.filter(function(s) {{
    return s.action.indexOf('exit') !== -1;
  }});
  if (exitSignals.length > 0) {{
    traces.push({{
      x: exitSignals.map(function(s) {{ return new Date(s.time); }}),
      y: exitSignals.map(function(s) {{ return s.price; }}),
      text: exitSignals.map(function(s) {{
        return s.action + (s.profit !== null ? ' P/L: ' + s.profit.toFixed(2) : '');
      }}),
      mode: 'markers', type: 'scatter', name: 'Exit',
      marker: {{ symbol: 'triangle-down', size: 12, color: '#ef5350' }}
    }});
  }}

  Plotly.newPlot('candlestick', traces, {{
    title: 'Price Chart',
    xaxis: {{ type: 'date', rangeslider: {{ visible: false }} }},
    yaxis: {{ title: 'Price' }},
    paper_bgcolor: '#1a1a2e', plot_bgcolor: '#16213e',
    font: {{ color: '#eee' }}
  }});

  // Volume bar chart
  var volColors = data.candles.map(function(c) {{
    return c.close >= c.open ? '#26a69a' : '#ef5350';
  }});
  Plotly.newPlot('volume', [{{
    x: times, y: volumes, type: 'bar', name: 'Volume',
    marker: {{ color: volColors }}
  }}], {{
    title: 'Volume', height: 200,
    xaxis: {{ type: 'date' }},
    yaxis: {{ title: 'Volume' }},
    paper_bgcolor: '#1a1a2e', plot_bgcolor: '#16213e',
    font: {{ color: '#eee' }}
  }});

  // Equity curve
  if (data.equity_curve.length > 0) {{
    var eqTimes = data.equity_curve.map(function(e) {{ return new Date(e.time); }});
    var eqVals  = data.equity_curve.map(function(e) {{ return e.equity; }});
    var ddVals  = data.equity_curve.map(function(e) {{ return e.drawdown_pct; }});
    Plotly.newPlot('equity', [
      {{ x: eqTimes, y: eqVals, type: 'scatter', mode: 'lines', name: 'Equity' }},
      {{ x: eqTimes, y: ddVals, type: 'scatter', mode: 'lines', name: 'Drawdown %',
         yaxis: 'y2', line: {{ color: '#ef5350' }} }}
    ], {{
      title: 'Equity Curve', height: 300,
      xaxis: {{ type: 'date' }},
      yaxis: {{ title: 'Equity', side: 'left' }},
      yaxis2: {{ title: 'Drawdown %', side: 'right', overlaying: 'y' }},
      paper_bgcolor: '#1a1a2e', plot_bgcolor: '#16213e',
      font: {{ color: '#eee' }}
    }});
  }}
}})();
</script>
</body>
</html>"#
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use ssm_core::Candle;

    fn sample_candles() -> Vec<Candle> {
        vec![
            Candle {
                open_time: 1_700_000_000_000,
                open: Decimal::from(100),
                high: Decimal::from(110),
                low: Decimal::from(95),
                close: Decimal::from(105),
                volume: Decimal::from(1000),
                close_time: 1_700_000_060_000,
                quote_volume: Decimal::from(100_000),
                trades: 50,
                taker_buy_volume: Decimal::from(600),
                taker_sell_volume: Decimal::from(400),
            },
            Candle {
                open_time: 1_700_000_060_000,
                open: Decimal::from(105),
                high: Decimal::from(115),
                low: Decimal::from(100),
                close: Decimal::from(112),
                volume: Decimal::from(1200),
                close_time: 1_700_000_120_000,
                quote_volume: Decimal::from(120_000),
                trades: 60,
                taker_buy_volume: Decimal::from(700),
                taker_sell_volume: Decimal::from(500),
            },
        ]
    }

    #[test]
    fn from_candles_creates_correct_points() {
        let candles = sample_candles();
        let chart = ChartData::from_candles(&candles);

        assert_eq!(chart.candles.len(), 2);
        assert_eq!(chart.candles[0].time, 1_700_000_000_000);
        assert!((chart.candles[0].open - 100.0).abs() < f64::EPSILON);
        assert!((chart.candles[0].high - 110.0).abs() < f64::EPSILON);
        assert!((chart.candles[0].low - 95.0).abs() < f64::EPSILON);
        assert!((chart.candles[0].close - 105.0).abs() < f64::EPSILON);
        assert!((chart.candles[0].volume - 1000.0).abs() < f64::EPSILON);
        assert!(chart.indicators.is_empty());
        assert!(chart.signals.is_empty());
        assert!(chart.equity_curve.is_empty());
    }

    #[test]
    fn to_json_produces_valid_json() {
        let candles = sample_candles();
        let chart = ChartData::from_candles(&candles);
        let json = chart.to_json();

        let parsed: serde_json::Value = serde_json::from_str(&json).expect("should be valid JSON");
        assert!(parsed["candles"].is_array());
        assert_eq!(parsed["candles"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn to_html_produces_html_with_plotly() {
        let candles = sample_candles();
        let chart = ChartData::from_candles(&candles);
        let html = chart.to_html();

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("plotly"));
        assert!(html.contains("candlestick"));
        assert!(html.contains("</html>"));
    }

    #[test]
    fn add_indicator_adds_to_list() {
        let mut chart = ChartData::from_candles(&sample_candles());
        assert!(chart.indicators.is_empty());

        chart.add_indicator(
            "EMA_20",
            ChartType::Line,
            &[(1_700_000_000_000, 102.5), (1_700_000_060_000, 108.0)],
        );

        assert_eq!(chart.indicators.len(), 1);
        assert_eq!(chart.indicators[0].name, "EMA_20");
        assert_eq!(chart.indicators[0].data.len(), 2);
    }

    #[test]
    fn add_signal_adds_to_list() {
        let mut chart = ChartData::from_candles(&sample_candles());
        assert!(chart.signals.is_empty());

        chart.add_signal(1_700_000_000_000, 105.0, "entry_long", None);
        chart.add_signal(1_700_000_060_000, 112.0, "exit_long", Some(7.0));

        assert_eq!(chart.signals.len(), 2);
        assert_eq!(chart.signals[0].action, "entry_long");
        assert!(chart.signals[0].profit.is_none());
        assert_eq!(chart.signals[1].action, "exit_long");
        assert!((chart.signals[1].profit.unwrap() - 7.0).abs() < f64::EPSILON);
    }

    #[test]
    fn candle_point_serialization() {
        let pt = CandlePoint {
            time: 1_700_000_000_000,
            open: 100.0,
            high: 110.0,
            low: 95.0,
            close: 105.0,
            volume: 1000.0,
        };
        let json = serde_json::to_string(&pt).expect("serialize");
        assert!(json.contains("\"time\":1700000000000"));
        assert!(json.contains("\"open\":100.0"));
        assert!(json.contains("\"volume\":1000.0"));
    }
}
