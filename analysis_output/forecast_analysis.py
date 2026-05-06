#!/usr/bin/env python3
"""门店活跃交易规模趋势分析与3年预测 - v3 (基于DWD层 ds_sharding)"""

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import matplotlib.ticker as mticker
import numpy as np
from datetime import datetime, timedelta
import json, os, warnings
warnings.filterwarnings('ignore')

# ── Data from SQL queries (DWD layer, ds_sharding as proxy for business date) ──
monthly_data = [
    # (mon, active_stores, new_stores, total_revenue)
    ("2023-05", 10, 10, 6948929.17),
    ("2023-06", 7, 1, 591401.94),
    ("2023-07", 6, 4, 344509.09),
    ("2023-08", 20, 14, 18999006.15),
    ("2023-09", 10, 0, 49549.44),
    ("2023-10", 14, 6, 113527.34),
    ("2023-11", 18, 6, 117205.83),
    ("2023-12", 62, 46, 1645320.16),
    ("2024-01", 168, 123, 10023852663.38),
    ("2024-02", 198, 65, 31686697.21),
    ("2024-03", 214, 41, 48829478.09),
    ("2024-04", 258, 67, 58112754.43),
    ("2024-05", 371, 139, 61857425.93),
    ("2024-06", 839, 519, 112130454.07),
    ("2024-07", 1444, 718, 251873154.46),
    ("2024-08", 1946, 690, 4112574546.90),
    ("2024-09", 2406, 666, 500214335.99),
    ("2024-10", 2981, 800, 670529407.42),
    ("2024-11", 3581, 851, 926506761.93),
    ("2024-12", 4191, 908, 14067599520.31),
    ("2025-01", 4736, 832, 1363555820.66),
    ("2025-02", 5116, 724, 2534229110.57),
    ("2025-03", 5708, 823, 11851394319.64),
    ("2025-04", 6074, 706, 12551300984.80),
    ("2025-05", 6653, 836, 52469809155.39),
    ("2025-06", 7240, 917, 52463790724.06),
    ("2025-07", 8005, 1091, 4075275451.26),
    ("2025-08", 8790, 1178, 3252639720.93),
    ("2025-09", 9366, 1057, 3997186227.58),
    ("2025-10", 10259, 1245, 3668708451.86),
    ("2025-11", 11023, 1181, 4542754603.03),
    ("2025-12", 11663, 1246, 4706953069.42),
    ("2026-01", 12413, 1238, 4518826653.79),
    ("2026-02", 12955, 1146, 7826951094.83),
    ("2026-03", 13881, 1374, 5871397852.19),
    ("2026-04", 14224, 1103, 5019716945.30),
    ("2026-05", 12690, 190, 762470358.85),  # partial month
]

# Parse
dates = [datetime.strptime(d[0], "%Y-%m") for d in monthly_data]
active = np.array([d[1] for d in monthly_data], dtype=float)
new_stores = np.array([d[2] for d in monthly_data], dtype=float)
revenue = np.array([d[3] for d in monthly_data], dtype=float)
months_idx = np.arange(len(active))

# Filter to recent stable-growth period (2024-07 onward, ~2 years)
cutoff = datetime(2024, 7, 1)
recent_mask = np.array([d >= cutoff for d in dates])
recent_idx = months_idx[recent_mask]
recent_active = active[recent_mask]
recent_new = new_stores[recent_mask]
recent_dates = [dates[i] for i in range(len(dates)) if recent_mask[i]]

# ── Forecasting Models ──

# Model 1: Linear trend on recent window
from numpy.polynomial import polynomial as P
coefs_linear = np.polyfit(recent_idx, recent_active, 1)
linear_fn = np.poly1d(coefs_linear)

# Model 2: Quadratic (accelerating growth)
coefs_quad = np.polyfit(recent_idx, recent_active, 2)
quad_fn = np.poly1d(coefs_quad)

# Model 3: Robust - new store rate based
# Recent 12-month average new store rate
recent_12_new = new_stores[-12:]
avg_new_rate = np.mean(recent_12_new)
std_new_rate = np.std(recent_12_new)

# Forecast horizon: 36 months (~3 years)
forecast_months = 36
last_idx = months_idx[-1]
forecast_idx = np.arange(last_idx + 1, last_idx + forecast_months + 1)

# Generate forecast dates
forecast_dates = []
d = dates[-1]
for i in range(1, forecast_months + 1):
    m = d.month + i
    y = d.year + (m - 1) // 12
    m = ((m - 1) % 12) + 1
    forecast_dates.append(datetime(y, m, 1))

# Central forecast: new stores per month with slight deceleration
# Assume new store rate decays 5%/year from recent avg
central_rates = []
for i in range(forecast_months):
    years_out = i / 12.0
    decay = max(0.5, 1.0 - 0.05 * years_out)  # min 50% of current rate
    central_rates.append(avg_new_rate * decay)

central_forecast = [active[-1]]
for r in central_rates:
    central_forecast.append(central_forecast[-1] + r)
central_forecast = central_forecast[1:]  # drop seed

# Upper bound: sustained new store rate (no decay)
upper_rates = [avg_new_rate + std_new_rate] * forecast_months
upper_forecast = [active[-1]]
for r in upper_rates:
    upper_forecast.append(upper_forecast[-1] + r)
upper_forecast = upper_forecast[1:]

# Lower bound: rapid decay (10%/year) from avg_new_rate - 1 std
lower_rate_start = max(avg_new_rate - std_new_rate, 500)
lower_rates = []
for i in range(forecast_months):
    years_out = i / 12.0
    decay = max(0.3, 1.0 - 0.10 * years_out)
    lower_rates.append(lower_rate_start * decay)

lower_forecast = [active[-1]]
for r in lower_rates:
    lower_forecast.append(lower_forecast[-1] + r)
lower_forecast = lower_forecast[1:]

# ── Linear fit forecast ──
linear_forecast = linear_fn(forecast_idx)
quad_forecast = quad_fn(forecast_idx)

# ── Revenue per store analysis ──
rev_per_store = revenue / active

# ── PLOTS ──
plt.rcParams.update({
    'font.family': 'DejaVu Sans',
    'figure.dpi': 150,
    'savefig.dpi': 150,
    'axes.titlesize': 14,
    'axes.labelsize': 11,
})

os.makedirs('analysis_output', exist_ok=True)

# === Figure 1: Active Stores Trend + 3-Year Forecast ===
fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(14, 12), gridspec_kw={'height_ratios': [2, 1]})

# Main plot
all_dates_plot = dates + forecast_dates
ax1.plot(dates, active, 'o-', color='#2196F3', linewidth=2, markersize=3, label='Monthly Active Stores (Actual)', zorder=3)

# Forecast lines
ax1.plot(forecast_dates, central_forecast, '-', color='#FF9800', linewidth=2.5, label='Central Forecast (new store rate w/ 5%/yr decay)')
ax1.fill_between(forecast_dates, lower_forecast, upper_forecast, alpha=0.15, color='#FF9800', label='Upper/Lower Bound Range')

ax1.plot(forecast_dates, linear_forecast, '--', color='#9C27B0', linewidth=1.5, alpha=0.7, label='Linear Trend (2024H2+)')
ax1.plot(forecast_dates, quad_forecast, ':', color='#E91E63', linewidth=1.5, alpha=0.7, label='Quadratic Trend')

# Annotations for key milestones
annotations = [(0, float(active[0])), (len(active)-1, float(active[-1]))]
if np.any(active > 5000):
    annotations.append((int(np.argmax(active > 5000)), 5000))
if np.any(active > 10000):
    annotations.append((int(np.argmax(active > 10000)), 10000))
for i, v in annotations:
    if i < len(dates):
        ax1.annotate(f'{int(v):,}', (dates[i], active[i]), textcoords="offset points",
                    xytext=(0, 12), ha='center', fontsize=8, color='#333')

# Forecast endpoint annotations
ax1.annotate(f'{int(central_forecast[-1]):,}', (forecast_dates[-1], central_forecast[-1]),
            textcoords="offset points", xytext=(10, 0), fontsize=9, fontweight='bold', color='#FF9800')
ax1.annotate(f'{int(upper_forecast[-1]):,}', (forecast_dates[-1], upper_forecast[-1]),
            textcoords="offset points", xytext=(10, 5), fontsize=8, color='#E65100')
ax1.annotate(f'{int(lower_forecast[-1]):,}', (forecast_dates[-1], lower_forecast[-1]),
            textcoords="offset points", xytext=(10, -10), fontsize=8, color='#BF360C')

ax1.set_ylabel('Active Stores')
ax1.set_title('Store Active Trading Trend & 3-Year Forecast (DWD ds_sharding)', fontweight='bold')
ax1.legend(loc='upper left', fontsize=8, framealpha=0.9)
ax1.grid(True, alpha=0.3)
ax1.yaxis.set_major_formatter(mticker.FuncFormatter(lambda x, _: f'{int(x):,}'))

# Vertical line separating actual vs forecast
ax1.axvline(x=dates[-1], color='gray', linestyle='--', alpha=0.5, linewidth=1)

# Bottom: New stores per month
colors_new = ['#4CAF50' if v > np.median(recent_new) else '#81C784' for v in new_stores]
ax2.bar(dates, new_stores, color=colors_new, alpha=0.8, label='New Stores/Month')

# Average new store rate line
ax2.axhline(y=avg_new_rate, color='#FF5722', linestyle='--', linewidth=1.5, 
           label=f'Recent 12-mo avg: {avg_new_rate:.0f}/mo')
ax2.axhline(y=avg_new_rate + std_new_rate, color='#E91E63', linestyle=':', linewidth=1, alpha=0.6)
ax2.axhline(y=avg_new_rate - std_new_rate, color='#E91E63', linestyle=':', linewidth=1, alpha=0.6)

ax2.set_ylabel('New Stores')
ax2.set_title('Monthly New Store Acquisitions')
ax2.legend(loc='upper left', fontsize=8)
ax2.grid(True, alpha=0.3, axis='y')
ax2.yaxis.set_major_formatter(mticker.FuncFormatter(lambda x, _: f'{int(x):,}'))

plt.tight_layout()
fig.savefig('analysis_output/forecast_active_stores.png', bbox_inches='tight')
plt.close()
print("✓ Saved: forecast_active_stores.png")

# === Figure 2: Revenue & Per-Store Economics ===
fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(14, 10))

# Total Revenue (log scale due to wide range)
ax1.bar(dates, revenue / 1e6, color='#3F51B5', alpha=0.7, label='Total Revenue (M)')
ax1_2 = ax1.twinx()
ax1_2.plot(dates, active, 'o-', color='#FF5722', linewidth=2, markersize=3, label='Active Stores')
ax1.set_ylabel('Total Revenue (M THB)')
ax1_2.set_ylabel('Active Stores')
ax1.set_title('Monthly Revenue & Active Stores', fontweight='bold')

# Combine legends
lines1, labels1 = ax1.get_legend_handles_labels()
lines2, labels2 = ax1_2.get_legend_handles_labels()
ax1.legend(lines1 + lines2, labels1 + labels2, loc='upper left', fontsize=8)
ax1.grid(True, alpha=0.3)

# Avg revenue per store (filter outliers: skip 2023 and extreme months)
valid_mask = (active > 50) & (rev_per_store < rev_per_store[active > 50].mean() * 3)
ax2.plot([dates[i] for i in range(len(dates)) if valid_mask[i]], 
         [rev_per_store[i] / 1000 for i in range(len(dates)) if valid_mask[i]],
         'o-', color='#009688', linewidth=2, markersize=3)
ax2.axhline(y=np.median(rev_per_store[valid_mask]) / 1000, color='#E91E63', linestyle='--', 
           label=f'Median: {np.median(rev_per_store[valid_mask])/1000:.0f}k THB')
ax2.set_ylabel('Avg Revenue per Store (k THB)')
ax2.set_xlabel('Month')
ax2.set_title('Per-Store Monthly Revenue (stores>50, outliers removed)', fontweight='bold')
ax2.legend(fontsize=8)
ax2.grid(True, alpha=0.3)
ax2.yaxis.set_major_formatter(mticker.FuncFormatter(lambda x, _: f'{int(x):,}k'))

plt.tight_layout()
fig.savefig('analysis_output/revenue_per_store.png', bbox_inches='tight')
plt.close()
print("✓ Saved: revenue_per_store.png")

# === Figure 3: Growth Rate Decomposition ===
fig, ax = plt.subplots(figsize=(14, 6))

# Month-over-month growth rate
mom_growth = np.diff(active) / active[:-1] * 100
mom_dates = dates[1:]
# Smooth with 3-month rolling
smooth_growth = np.convolve(mom_growth, np.ones(3)/3, mode='valid')
smooth_dates = mom_dates[1:-1]

ax.plot(smooth_dates, smooth_growth, '-', color='#673AB7', linewidth=2, label='MoM Growth Rate (3-mo smoothed)')
ax.axhline(y=0, color='gray', linewidth=0.5)

# Mark phases
ax.axvspan(dates[0], datetime(2024,6,30), alpha=0.08, color='blue', label='Early/Ramp-up')
ax.axvspan(datetime(2024,7,1), datetime(2025,6,30), alpha=0.08, color='green', label='Hyper-growth')
ax.axvspan(datetime(2025,7,1), dates[-1], alpha=0.08, color='orange', label='Stable Growth')

ax.set_ylabel('Month-over-Month Growth Rate (%)')
ax.set_xlabel('Month')
ax.set_title('Active Store Growth Rate Analysis', fontweight='bold')
ax.legend(loc='upper right', fontsize=8)
ax.grid(True, alpha=0.3)

plt.tight_layout()
fig.savefig('analysis_output/growth_rate.png', bbox_inches='tight')
plt.close()
print("✓ Saved: growth_rate.png")

# ── Output Summary Stats ──
print("\n" + "="*60)
print("FORECAST SUMMARY (Based on DWD ds_sharding, 2023-05 to 2026-05)")
print("="*60)
print(f"\nData: {len(active)} months of data, {active[-1]:,.0f} active stores as of {dates[-1].strftime('%Y-%m')}")
print(f"Recent 12-mo avg new stores/month: {avg_new_rate:.0f} (±{std_new_rate:.0f})")
print(f"\n--- 3-Year Forecast ---")
print(f"{'Horizon':<15} {'Lower':>10} {'Central':>10} {'Upper':>10}")
for i in [6, 12, 24, 36]:
    idx = i - 1
    print(f"{i:>3} months ({forecast_dates[idx].strftime('%Y-%m')}): {lower_forecast[idx]:>10,.0f} {central_forecast[idx]:>10,.0f} {upper_forecast[idx]:>10,.0f}")

# Revenue per store stability check
recent_rev_per = rev_per_store[-12:][active[-12:] > 50]
if len(recent_rev_per) > 3:
    print(f"\nPer-store revenue (recent 12mo, stores>50):")
    print(f"  Median: {np.median(recent_rev_per):,.0f} THB")
    print(f"  Mean:   {np.mean(recent_rev_per):,.0f} THB")
    print(f"  CV:     {np.std(recent_rev_per)/np.mean(recent_rev_per)*100:.1f}%")

# Growth rate analysis
recent_6mo_growth = (active[-1] / active[-7] - 1) * 100
recent_12mo_growth = (active[-1] / active[-13] - 1) * 100
print(f"\nGrowth metrics:")
print(f"  6-month growth: {recent_6mo_growth:.1f}% (annualized: {(((1+recent_6mo_growth/100)**2 - 1)*100):.1f}%)")
print(f"  12-month growth: {recent_12mo_growth:.1f}%")

# Save forecast data as JSON for report
forecast_json = {
    "data_period": {"start": "2023-05", "end": "2026-05"},
    "current_active_stores": int(active[-1]),
    "avg_new_stores_per_month": float(avg_new_rate),
    "std_new_stores": float(std_new_rate),
    "forecast": {
        "forecast_horizon_months": 36,
        "forecast_end_date": forecast_dates[-1].strftime("%Y-%m"),
        "lower_end": int(lower_forecast[-1]),
        "central_end": int(central_forecast[-1]),
        "upper_end": int(upper_forecast[-1]),
        "monthly_detail": []
    }
}
for i in [6, 12, 18, 24, 30, 36]:
    idx = i - 1
    forecast_json["forecast"]["monthly_detail"].append({
        "months_out": i,
        "date": forecast_dates[idx].strftime("%Y-%m"),
        "lower": int(lower_forecast[idx]),
        "central": int(central_forecast[idx]),
        "upper": int(upper_forecast[idx])
    })

with open('analysis_output/forecast_summary.json', 'w') as f:
    json.dump(forecast_json, f, indent=2)
print("\n✓ Saved: forecast_summary.json")
print("\nDone! All analysis artifacts ready.")
