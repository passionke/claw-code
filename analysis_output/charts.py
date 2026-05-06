#!/usr/bin/env python3
"""Store scale and 3-year forecast analysis — English labels for font compat"""
import os
import numpy as np
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import matplotlib.ticker as mticker

# === RAW DATA FROM SQL QUERIES ===
daily_data = {
    "2026-04-26": {"orders": 388390, "revenue": 181924912.93, "diners": 281130, "stores": 10598},
    "2026-04-27": {"orders": 360897, "revenue": 153583803.47, "diners": 222615, "stores": 10903},
    "2026-04-28": {"orders": 363108, "revenue": 163875147.69, "diners": 218547, "stores": 11329},
    "2026-04-29": {"orders": 240223, "revenue": 97577959.31, "diners": 113444, "stores": 10429},
}
store_active_days = {1: 783, 2: 975, 3: 3242, 4: 7700}
total_stores_dim = 31882
stores_with_trade = 12700
payment = {
    "CASH": {"stores": 11959, "amount": 370347660.29},
    "CAB": {"stores": 8891, "amount": 350370953.68},
    "ONLINE": {"stores": 1616, "amount": 23078278.45},
    "MEMBER": {"stores": 107, "amount": 11865927.52},
}
hourly_orders = {
    13: 155692, 14: 135783, 15: 118784, 19: 118149, 12: 118752,
    16: 112809, 20: 110816, 18: 109684, 17: 107931, 21: 86230,
    11: 99278, 10: 85295, 22: 57613, 9: 72805, 8: 47869,
    7: 19767, 23: 36865, 0: 35824, 1: 20806, 2: 12931,
    3: 9664, 4: 7588, 5: 6086, 6: 7904,
}

# === DERIVED METRICS ===
daily_avg_orders = np.mean([d['orders'] for d in daily_data.values()])
daily_avg_revenue = np.mean([d['revenue'] for d in daily_data.values()])
daily_avg_stores = np.mean([d['stores'] for d in daily_data.values()])
orders_per_store = daily_avg_orders / daily_avg_stores
revenue_per_store = daily_avg_revenue / daily_avg_stores
aov = daily_avg_revenue / daily_avg_orders
stores_pending = total_stores_dim - stores_with_trade

# === 3-YEAR FORECAST ===
months = np.arange(0, 37)

def forecast_stores(annual_growth, pending_months, pending_ratio, base, pending, m):
    organic = base * ((1 + annual_growth)**(m/12) - 1)
    if pending_months > 0:
        pending_act = pending * pending_ratio * min(1.0, m / pending_months)
    else:
        pending_act = 0
    return base + organic + pending_act

low_stores = [forecast_stores(0.05, 36, 0.80, stores_with_trade, stores_pending, m) for m in months]
mid_stores = [forecast_stores(0.15, 24, 1.0, stores_with_trade, stores_pending, m) for m in months]
high_stores = [forecast_stores(0.25, 12, 1.0, stores_with_trade, stores_pending, m) for m in months]

# === CHART 1: Daily Snapshot ===
fig, axes = plt.subplots(1, 3, figsize=(16, 5))
day_labels = ['4/26\n(Sun)', '4/27\n(Mon)', '4/28\n(Tue)', '4/29\n(Wed)']
colors = ['#2196F3', '#4CAF50', '#FF9800', '#9C27B0']

orders_vals = [d['orders'] for d in daily_data.values()]
axes[0].bar(day_labels, orders_vals, color=colors, alpha=0.85)
axes[0].set_title('Daily Orders', fontsize=14, fontweight='bold')
axes[0].set_ylabel('Orders')
axes[0].yaxis.set_major_formatter(mticker.FuncFormatter(lambda x, _: f'{x/1000:.0f}k'))
for i, v in enumerate(orders_vals):
    axes[0].text(i, v+5000, f'{v/1000:.0f}k', ha='center', fontsize=9)

stores_vals = [d['stores'] for d in daily_data.values()]
axes[1].bar(day_labels, stores_vals, color=colors, alpha=0.85)
axes[1].set_title('Daily Active Stores', fontsize=14, fontweight='bold')
for i, v in enumerate(stores_vals):
    axes[1].text(i, v+100, f'{v:,}', ha='center', fontsize=9)

rev_vals = [d['revenue']/1e6 for d in daily_data.values()]
axes[2].bar(day_labels, rev_vals, color=colors, alpha=0.85)
axes[2].set_title('Daily Revenue (Million)', fontsize=14, fontweight='bold')
for i, v in enumerate(rev_vals):
    axes[2].text(i, v+1, f'{v:.0f}M', ha='center', fontsize=9)

plt.tight_layout()
plt.savefig('analysis_output/chart1_daily_snapshot.png', dpi=150, bbox_inches='tight')
plt.close()
print("Chart 1 saved")

# === CHART 2: 3-Year Store Forecast ===
fig, ax = plt.subplots(figsize=(14, 7))
ax.fill_between(months, low_stores, high_stores, alpha=0.12, color='#4CAF50',
                label='Forecast Range (Conservative ~ Optimistic)')
ax.plot(months, mid_stores, 'b-', linewidth=3,
        label=f'Baseline (15% annual growth)', marker='o', markersize=3, markevery=3)
ax.plot(months, high_stores, 'r--', linewidth=2,
        label=f'Optimistic (25% annual growth)', marker='s', markersize=3, markevery=3)
ax.plot(months, low_stores, 'g--', linewidth=2,
        label=f'Conservative (5% annual growth)', marker='^', markersize=3, markevery=3)

ax.axhline(y=total_stores_dim, color='orange', linestyle=':', linewidth=1.5,
           label=f'Registered Stores ({total_stores_dim:,})')
ax.axhline(y=stores_with_trade, color='gray', linestyle='-.', linewidth=1,
           label=f'Current Active (M0: {stores_with_trade:,})')

ax.set_xlabel('Month (M0=2026-05)', fontsize=12)
ax.set_ylabel('Active Stores', fontsize=12)
ax.set_title('Active Store Scale — 3-Year Forecast', fontsize=16, fontweight='bold')
ax.legend(loc='upper left', fontsize=9)
ax.yaxis.set_major_formatter(mticker.FuncFormatter(lambda x, _: f'{x/1000:.0f}k'))
ax.set_xlim(-0.5, 37)
ax.grid(True, alpha=0.3)
ax.set_xticks(range(0, 37, 3))

# Annotations
for (x, v, label, color) in [(12, high_stores[12], 'Y1 Hi', '#D84315'),
                                (24, mid_stores[24], 'Y2 Mid', '#1565C0'),
                                (36, mid_stores[36], 'Y3 Mid', '#1565C0')]:
    ax.annotate(f'{label}\n{v:,.0f}', xy=(x, v), xytext=(x, v*1.05),
                fontsize=9, ha='center', color=color, fontweight='bold')

plt.tight_layout()
plt.savefig('analysis_output/chart2_store_forecast.png', dpi=150, bbox_inches='tight')
plt.close()
print("Chart 2 saved")

# === CHART 3: Revenue Forecast ===
fig, ax = plt.subplots(figsize=(14, 7))
low_rev = [s * revenue_per_store * 30 / 1e9 for s in low_stores]
mid_rev = [s * revenue_per_store * 30 / 1e9 for s in mid_stores]
high_rev = [s * revenue_per_store * 30 / 1e9 for s in high_stores]

ax.fill_between(months, low_rev, high_rev, alpha=0.12, color='#FF9800',
                label='Forecast Range')
ax.plot(months, mid_rev, 'b-', linewidth=3, label='Baseline')
ax.plot(months, high_rev, 'r--', linewidth=2, label='Optimistic')
ax.plot(months, low_rev, 'g--', linewidth=2, label='Conservative')

ax.set_xlabel('Month', fontsize=12)
ax.set_ylabel('Monthly Revenue (Billion)', fontsize=12)
ax.set_title('Monthly Revenue — 3-Year Forecast (per-store quality constant)',
             fontsize=16, fontweight='bold')
ax.legend(loc='upper left', fontsize=10)
ax.set_xlim(-0.5, 37)
ax.grid(True, alpha=0.3)
ax.set_xticks(range(0, 37, 3))
ax.yaxis.set_major_formatter(mticker.FuncFormatter(lambda x, _: f'{x:.1f}B'))

for (x, v, label) in [(12, high_rev[12], 'Y1 Hi'), (24, mid_rev[24], 'Y2 Mid'),
                         (36, mid_rev[36], 'Y3 Mid')]:
    ax.annotate(f'{label}: {v:.1f}B', xy=(x, v), xytext=(x+1, v*1.02),
                fontsize=9, ha='center', color='#D84315', fontweight='bold')

plt.tight_layout()
plt.savefig('analysis_output/chart3_revenue_forecast.png', dpi=150, bbox_inches='tight')
plt.close()
print("Chart 3 saved")

# === CHART 4: Hourly Pattern ===
fig, ax = plt.subplots(figsize=(14, 5))
hours = list(range(24))
orders_by_hour = [hourly_orders.get(h, 0) for h in hours]
bar_colors = ['#FF5722' if h in [12,13,14,18,19,20] else '#607D8B' for h in hours]
ax.bar(hours, orders_by_hour, color=bar_colors, alpha=0.85, width=0.8)
ax.set_xlabel('Hour', fontsize=12)
ax.set_ylabel('Orders', fontsize=12)
ax.set_title('24-Hour Transaction Distribution (4-Day Aggregate)', fontsize=14, fontweight='bold')
ax.set_xticks(range(0, 24, 2))
ax.yaxis.set_major_formatter(mticker.FuncFormatter(lambda x, _: f'{x/1000:.0f}k'))

ax.annotate('Lunch Peak\n12-15', xy=(13.5, 155692), xytext=(14.5, 160000),
            fontsize=10, ha='center', color='#D84315',
            arrowprops=dict(arrowstyle='->', color='#D84315', lw=1.5))
ax.annotate('Dinner Peak\n18-20', xy=(19, 118149), xytext=(20.5, 135000),
            fontsize=10, ha='center', color='#D84315',
            arrowprops=dict(arrowstyle='->', color='#D84315', lw=1.5))

plt.tight_layout()
plt.savefig('analysis_output/chart4_hourly_pattern.png', dpi=150, bbox_inches='tight')
plt.close()
print("Chart 4 saved")

# === CHART 5: Store Activity Distribution ===
fig, ax = plt.subplots(figsize=(10, 6))
days_labels = list(store_active_days.keys())
counts = [store_active_days[d] for d in days_labels]
pie_colors = ['#FF5722', '#FF9800', '#FFC107', '#4CAF50']
wedges, texts, autotexts = ax.pie(counts, explode=(0,0,0,0.05),
    labels=[f'{d} Day{"s" if d>1 else ""}' for d in days_labels],
    autopct='%1.1f%%', colors=pie_colors, startangle=90, textprops={'fontsize': 12})
for t in autotexts:
    t.set_fontweight('bold')
ax.set_title(f'Store Activity Distribution (Total: {stores_with_trade:,} stores)',
             fontsize=14, fontweight='bold')
plt.tight_layout()
plt.savefig('analysis_output/chart5_activity_distribution.png', dpi=150, bbox_inches='tight')
plt.close()
print("Chart 5 saved")

# === CHART 6: Payment Distribution ===
fig, ax = plt.subplots(figsize=(10, 6))
pay_labels = list(payment.keys())
pay_amounts = [payment[k]['amount']/1e6 for k in pay_labels]
pay_colors = ['#4CAF50', '#2196F3', '#FF9800', '#9C27B0']
bars = ax.bar(pay_labels, pay_amounts, color=pay_colors, alpha=0.85)
ax.set_ylabel('Amount (Million)', fontsize=12)
ax.set_title('Payment Method Distribution by Amount', fontsize=14, fontweight='bold')
total_pay = sum(pay_amounts)
for bar, amt in zip(bars, pay_amounts):
    ax.text(bar.get_x() + bar.get_width()/2, bar.get_height() + 2,
            f'{amt:.0f}M\n({amt/total_pay*100:.1f}%)', ha='center', fontsize=10)
plt.tight_layout()
plt.savefig('analysis_output/chart6_payment.png', dpi=150, bbox_inches='tight')
plt.close()
print("Chart 6 saved")

# === Summary output ===
print("\n=== ANALYSIS SUMMARY ===")
print(f"Registered stores (dim): {total_stores_dim:,}")
print(f"Active trading stores:  {stores_with_trade:,}")
print(f"Pending activation:     {stores_pending:,}")
print(f"Daily avg orders:       {daily_avg_orders:,.0f}")
print(f"Daily avg revenue:      {daily_avg_revenue:,.0f}")
print(f"Avg orders/store/day:   {orders_per_store:.1f}")
print(f"Avg revenue/store/day:  {revenue_per_store:,.0f}")
print(f"Average Order Value:    {aov:,.0f}")

print("\n=== 3-YEAR FORECAST ===")
print(f"{'Scenario':<20} {'M12 Stores':<15} {'M24 Stores':<15} {'M36 Stores':<15} {'M36 Monthly Rev'}")
for name, data in [("Conservative (5%)", low_stores), ("Baseline (15%)", mid_stores), ("Optimistic (25%)", high_stores)]:
    rev36 = data[36] * revenue_per_store * 30 / 1e9
    print(f"{name:<20} {data[12]:<15,.0f} {data[24]:<15,.0f} {data[36]:<15,.0f} {rev36:.2f}B")

print("\nDone! All charts saved to analysis_output/")
