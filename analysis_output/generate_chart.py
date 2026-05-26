#!/usr/bin/env python3
"""Active Store Trend Analysis & 3-Year Forecast"""
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import numpy as np
from datetime import datetime

# ── Chinese font support ──
plt.rcParams['font.sans-serif'] = ['Arial Unicode MS', 'SimHei', 'DejaVu Sans']
plt.rcParams['axes.unicode_minus'] = False

# ══════════════════════════════════════════════════
# RAW DATA (from SQL queries)
# ══════════════════════════════════════════════════

# New store openings per month (extracted from store_id date prefix)
new_stores = {
    '2023-05': 13, '2023-06': 2, '2023-07': 6, '2023-08': 26,
    '2023-09': 3, '2023-10': 6, '2023-11': 14, '2023-12': 140,
    '2024-01': 172, '2024-02': 66, '2024-03': 84, '2024-04': 126,
    '2024-05': 232, '2024-06': 824, '2024-07': 942, '2024-08': 961,
    '2024-09': 1075, '2024-10': 1134, '2024-11': 1151, '2024-12': 1209,
    '2025-01': 733, '2025-02': 5,  # Feb likely incomplete
}

# Current snapshot (April 2026)
ACTIVE_STORES_NOW = 12700
TOTAL_STORES_DIM  = 31874
MONTHLY_REVENUE   = 596961823.4   # 4 days of April data
MONTHLY_ORDERS    = 1352618       # 4 days of April data

# Derived: per-store daily metrics (4 days, 12700 stores)
AVG_DAILY_REVENUE_PER_STORE = MONTHLY_REVENUE / 12700 / 4   # ≈ 11,752
AVG_DAILY_ORDER_PER_STORE   = MONTHLY_ORDERS / 12700 / 4    # ≈ 26.6

# ══════════════════════════════════════════════════
# SCENARIO PARAMETERS
# ══════════════════════════════════════════════════
# Observed monthly new-store rate (2024 H2 average)
avg_monthly_new_observed = (824+942+961+1075+1134+1151+1209)/7  # ≈ 1042

# Monthly churn (estimated: ~2% monthly based on typical SaaS/retail)
CHURN_RATE = 0.02

# Forecast horizon: 36 months (May 2026 → Apr 2029)
forecast_months = 36
start_active = ACTIVE_STORES_NOW  # 12,700

# ── Three scenarios ──
# Baseline: maintain current ~1000 new/month, 2% churn
# Upper: 1200 new/month, 1.5% churn (optimistic expansion)
# Lower: 600 new/month, 3% churn (conservative / macro slowdown)

scenarios = {
    'upper':     {'new_per_month': 1200, 'churn': 0.015, 'label': '上界（乐观）', 'color': '#2ecc71'},
    'baseline':  {'new_per_month': 1042, 'churn': 0.020, 'label': '基准',        'color': '#3498db'},
    'lower':     {'new_per_month': 600,  'churn': 0.030, 'label': '下界（保守）', 'color': '#e74c3c'},
}

# ══════════════════════════════════════════════════
# SIMULATE
# ══════════════════════════════════════════════════
results = {}
for name, params in scenarios.items():
    stores = [start_active]
    for m in range(forecast_months):
        churned = stores[-1] * params['churn']
        net = stores[-1] - churned + params['new_per_month']
        stores.append(net)
    results[name] = stores

# Time axis
months_hist = ['2024-01','2024-04','2024-07','2024-10','2025-01','2025-04',
               '2025-07','2025-10','2026-01','2026-04']
hist_active = [700, 900, 2600, 4800, 7200, 8500, 9800, 10800, 11800, 12700]
# (Approximate from new store accumulation pattern)

forecast_labels = []
from datetime import datetime, timedelta
d = datetime(2026, 5, 1)
for i in range(forecast_months+1):
    forecast_labels.append(d.strftime('%Y-%m'))
    if d.month == 12:
        d = datetime(d.year+1, 1, 1)
    else:
        d = datetime(d.year, d.month+1, 1)

# ══════════════════════════════════════════════════
# FIGURE 1: Active Stores 3-Year Forecast
# ══════════════════════════════════════════════════
fig, axes = plt.subplots(2, 2, figsize=(18, 14))
fig.suptitle('活跃交易门店趋势分析与未来3年预测\n(数据截至2026年4月)', fontsize=16, fontweight='bold', y=0.98)

# ── Panel 1: New Store Openings ──
ax1 = axes[0, 0]
ns_months = sorted(new_stores.keys())
ns_values = [new_stores[m] for m in ns_months]
bars = ax1.bar(range(len(ns_months)), ns_values, color='#9b59b6', alpha=0.85)
ax1.set_xticks(range(len(ns_months)))
ax1.set_xticklabels(ns_months, rotation=45, ha='right', fontsize=8)
ax1.set_title('每月新开门店数量', fontsize=13, fontweight='bold')
ax1.set_ylabel('新开门店数')
# annotate last bar
for i, v in enumerate(ns_values):
    if v > 500:
        ax1.text(i, v+20, str(v), ha='center', fontsize=7, fontweight='bold')

# ── Panel 2: 3-Year Forecast Fan ──
ax2 = axes[0, 1]
x_idx = np.arange(len(forecast_labels))
for name, params in scenarios.items():
    ax2.plot(x_idx, results[name], label=params['label'],
             color=params['color'], linewidth=2.5)
ax2.fill_between(x_idx, results['lower'], results['upper'],
                 alpha=0.15, color='#3498db', label='预测区间')
# mark key thresholds
ax2.axhline(y=20000, color='gray', linestyle='--', alpha=0.5)
ax2.text(1, 20500, '20,000 门店里程碑', fontsize=9, color='gray')
ax2.axhline(y=30000, color='gray', linestyle='--', alpha=0.5)
ax2.text(1, 30500, '30,000 门店里程碑', fontsize=9, color='gray')
# x-axis labels (show every 6 months)
tick_pos = [i for i in range(0, len(forecast_labels), 6)]
tick_lab = [forecast_labels[i] for i in tick_pos]
ax2.set_xticks(tick_pos)
ax2.set_xticklabels(tick_lab, rotation=45, ha='right')
ax2.set_title('未来3年活跃门店数预测（含上下界）', fontsize=13, fontweight='bold')
ax2.set_ylabel('活跃门店数')
ax2.legend(loc='upper left', fontsize=9)
ax2.grid(True, alpha=0.3)

# ── Panel 3: Revenue Forecast ──
ax3 = axes[1, 0]
# Monthly revenue per store (current): 596M / 12700 ≈ 47,000 per store-month
rev_per_store_month = MONTHLY_REVENUE / ACTIVE_STORES_NOW
for name, params in scenarios.items():
    rev = [s * rev_per_store_month / 1e6 for s in results[name]]  # in millions
    ax3.plot(x_idx, rev, label=params['label'],
             color=params['color'], linewidth=2.5)
ax3.fill_between(x_idx,
                 [s * rev_per_store_month / 1e6 for s in results['lower']],
                 [s * rev_per_store_month / 1e6 for s in results['upper']],
                 alpha=0.15, color='#e67e22')
ax3.set_xticks(tick_pos)
ax3.set_xticklabels(tick_lab, rotation=45, ha='right')
ax3.set_title('未来3年营收规模预测（百万泰铢/月）', fontsize=13, fontweight='bold')
ax3.set_ylabel('月营收（百万）')
ax3.legend(loc='upper left', fontsize=9)
ax3.grid(True, alpha=0.3)

# ── Panel 4: Quarterly Summary Table ──
ax4 = axes[1, 1]
ax4.axis('off')
table_data = []
table_data.append(['季度', '基准（门店数）', '上界', '下界', '基准营收(M)'])
for q_start in range(0, forecast_months+1, 3):
    q_label = forecast_labels[q_start]
    base = int(results['baseline'][q_start])
    upper = int(results['upper'][q_start])
    lower = int(results['lower'][q_start])
    rev = f"{base * rev_per_store_month / 1e6:.0f}"
    table_data.append([q_label, f'{base:,}', f'{upper:,}', f'{lower:,}', rev])

table = ax4.table(cellText=table_data, loc='center', cellLoc='center',
                  colWidths=[0.18, 0.22, 0.2, 0.2, 0.2])
table.auto_set_font_size(False)
table.set_fontsize(9)
table.scale(1.0, 1.5)
# style header row
for j in range(5):
    table[0, j].set_facecolor('#3498db')
    table[0, j].set_text_props(color='white', fontweight='bold')
ax4.set_title('季度预测摘要', fontsize=13, fontweight='bold', pad=20)

plt.tight_layout(rect=[0, 0, 1, 0.95])
plt.savefig('/Users/sm4645/work/claw-code/analysis_output/store_trend_forecast.png',
            dpi=150, bbox_inches='tight', facecolor='white')
print("Chart saved to analysis_output/store_trend_forecast.png")

# ══════════════════════════════════════════════════
# FIGURE 2: Store Quality Analysis
# ══════════════════════════════════════════════════
fig2, axes2 = plt.subplots(1, 2, figsize=(16, 6))
fig2.suptitle('门店规模与质量分析', fontsize=14, fontweight='bold')

# Monthly new stores grouped by quarter
ax = axes2[0]
quarters = {}
for m, v in new_stores.items():
    year = m[:4]
    month = int(m[5:7])
    q = f"{year}-Q{(month-1)//3+1}"
    quarters[q] = quarters.get(q, 0) + v
q_labels = sorted(quarters.keys())
q_values = [quarters[q] for q in q_labels]
colors = plt.cm.Purples(np.linspace(0.4, 0.9, len(q_labels)))
ax.bar(range(len(q_labels)), q_values, color=colors)
ax.set_xticks(range(len(q_labels)))
ax.set_xticklabels(q_labels, rotation=45, ha='right', fontsize=9)
ax.set_title('季度新增门店数', fontsize=12, fontweight='bold')
ax.set_ylabel('新增门店数')
for i, v in enumerate(q_values):
    ax.text(i, v+20, str(v), ha='center', fontsize=9, fontweight='bold')

# Store active vs total
ax = axes2[1]
categories = ['维度表总门店', '活跃交易门店\n(2026-04)', '流失/沉睡门店']
values = [TOTAL_STORES_DIM, ACTIVE_STORES_NOW, TOTAL_STORES_DIM - ACTIVE_STORES_NOW]
colors2 = ['#95a5a6', '#2ecc71', '#e74c3c']
bars = ax.bar(categories, values, color=colors2, width=0.5)
for bar, val in zip(bars, values):
    ax.text(bar.get_x() + bar.get_width()/2, bar.get_height() + 300,
            f'{val:,}', ha='center', fontsize=11, fontweight='bold')
ax.set_title('门店转化漏斗', fontsize=12, fontweight='bold')
ax.set_ylabel('门店数')
ax.grid(axis='y', alpha=0.3)

plt.tight_layout()
plt.savefig('/Users/sm4645/work/claw-code/analysis_output/store_quality_analysis.png',
            dpi=150, bbox_inches='tight', facecolor='white')
print("Chart saved to analysis_output/store_quality_analysis.png")

# ══════════════════════════════════════════════════
# Print summary for report
# ══════════════════════════════════════════════════
print("\n" + "="*60)
print("FORECAST SUMMARY")
print("="*60)
for q_start in [0, 12, 24, 36]:
    if q_start <= forecast_months:
        label = forecast_labels[q_start]
        base = int(results['baseline'][q_start])
        upper = int(results['upper'][q_start])
        lower = int(results['lower'][q_start])
        rev_base = base * rev_per_store_month / 1e6
        print(f"{label}: 基准={base:,}  上界={upper:,}  下界={lower:,}  基准月营收≈{rev_base:,.0f}M")
print(f"\n当前（2026-04）: {ACTIVE_STORES_NOW:,} 活跃门店")
print(f"维度表总门店: {TOTAL_STORES_DIM:,}")
print(f"店均日营收: {AVG_DAILY_REVENUE_PER_STORE:,.0f}")
print(f"店均日订单: {AVG_DAILY_ORDER_PER_STORE:.1f}")
print(f"观测期新店月均新增: {avg_monthly_new_observed:.0f}")
