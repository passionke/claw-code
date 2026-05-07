#!/usr/bin/env python3
"""
交易活跃门店3年趋势预测图表生成
"""
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import matplotlib.ticker as mticker
import numpy as np
import os

# Set Chinese font support
plt.rcParams['font.sans-serif'] = ['Arial Unicode MS', 'SimHei', 'DejaVu Sans']
plt.rcParams['axes.unicode_minus'] = False

# ==================== DATA ====================
# Monthly new store onboarding (from SQLBot data)
new_stores_2024 = [172, 66, 84, 126, 232, 824, 942, 961, 1075, 1134, 1151, 1209]
new_stores_2025 = [443, 1192, 3603, 925, 1053, 1074, 1536, 1394, 1342, 1560, 1402, 1504]
new_stores_2026 = [1516, 1348, 1642, 1268]  # Jan-Apr 2026

# Current baseline data (2026-04)
daily_active_stores = [11380, 10598, 10903, 11329, 10429]
daily_total_amount = [206248499.52, 181924912.93, 153583803.47, 163875147.69, 97577959.31]
daily_total_orders = [389251, 388390, 360897, 363108, 240223]

avg_active_stores = np.mean(daily_active_stores)  # ~10,928
avg_daily_amount = np.mean(daily_total_amount)     # ~160M
avg_daily_orders = np.mean(daily_total_orders)     # ~348K
avg_amount_per_store = avg_daily_amount / avg_active_stores  # ~14,640
avg_orders_per_store = avg_daily_orders / avg_active_stores   # ~31.8

# Total registered stores
total_2024 = sum(new_stores_2024)  # 7,976
total_2025 = sum(new_stores_2025)  # 17,028
total_2026_ytd = sum(new_stores_2026) + 78  # 5,852 (including partial May)
other_stores = 956
total_registered = total_2024 + total_2025 + total_2026_ytd + other_stores  # ~31,812

# Monthly new store addition rate (recent 6 months, excl. Mar 2025 spike)
recent_monthly_avg = np.mean([1560, 1402, 1504, 1516, 1348, 1642, 1268])  # ~1,463

# Current activation rate
activation_rate = avg_active_stores / total_registered  # ~34.4%

print(f"=== Current Snapshot (2026-04) ===")
print(f"Total registered stores: {total_registered:,}")
print(f"Avg daily active stores: {avg_active_stores:,.0f}")
print(f"Activation rate: {activation_rate:.1%}")
print(f"Avg daily transaction amount: {avg_daily_amount:,.0f}")
print(f"Avg daily order count: {avg_daily_orders:,.0f}")
print(f"Avg amount per store: {avg_amount_per_store:,.0f}")
print(f"Avg orders per store: {avg_orders_per_store:,.0f}")
print(f"Monthly new store rate (recent): {recent_monthly_avg:,.0f}")

# ==================== FORECAST MODEL ====================
# Assumptions:
# - Monthly new stores: 1,400/month (base case)
# - New store quality unchanged (same avg transaction per store)
# - Activation rate: gradual improvement from 34% to 40% over 3 years
# - Natural churn: ~2% of active stores per month
# - Upper bound: monthly new stores +20%, faster activation growth
# - Lower bound: monthly new stores -20%, slower activation growth

months_forecast = 36  # 3 years
start_month = "2026-05"
base_monthly_new = 1463
churn_rate = 0.02  # 2% monthly churn of active stores

def forecast_scenario(monthly_new_base, activation_start, activation_end, churn, months):
    """Forecast active stores and transaction volume"""
    cumulative_stores = total_registered
    active = avg_active_stores
    
    active_stores_list = []
    total_stores_list = []
    monthly_amount_list = []
    monthly_orders_list = []
    activation_list = []
    
    for m in range(months):
        # New stores join
        new_stores = monthly_new_base
        cumulative_stores += new_stores
        
        # Activation rate gradually changes
        t = m / months
        current_activation = activation_start + (activation_end - activation_start) * t
        
        # Target active stores based on cumulative and activation rate
        target_active = cumulative_stores * current_activation
        
        # Churn from active
        churned = active * churn
        
        # Active = previous + new activations - churn
        new_activations = max(0, target_active - active + churned)
        active = active + new_activations - churned
        active = max(active, 1000)  # floor
        
        # Transaction volume (assuming per-store quality unchanged)
        daily_amount = active * avg_amount_per_store
        monthly_amount = daily_amount * 30
        
        active_stores_list.append(active)
        total_stores_list.append(cumulative_stores)
        monthly_amount_list.append(monthly_amount / 1e8)  # in 亿
        monthly_orders_list.append(active * avg_orders_per_store * 30)
        activation_list.append(active / cumulative_stores)
    
    return {
        'active_stores': active_stores_list,
        'total_stores': total_stores_list,
        'monthly_amount': monthly_amount_list,
        'monthly_orders': monthly_orders_list,
        'activation_rate': activation_list
    }

# Three scenarios
base = forecast_scenario(
    monthly_new_base=1463,
    activation_start=0.344,
    activation_end=0.40,
    churn=0.02,
    months=36
)

upper = forecast_scenario(
    monthly_new_base=1463 * 1.2,
    activation_start=0.344,
    activation_end=0.48,
    churn=0.015,
    months=36
)

lower = forecast_scenario(
    monthly_new_base=1463 * 0.8,
    activation_start=0.344,
    activation_end=0.36,
    churn=0.025,
    months=36
)

# Generate month labels
import datetime
month_labels = []
start = datetime.date(2026, 5, 1)
for i in range(36):
    d = datetime.date(start.year + (start.month - 1 + i) // 12, 
                      (start.month - 1 + i) % 12 + 1, 1)
    month_labels.append(d.strftime('%Y-%m'))

# ==================== CHARTS ====================
fig, axes = plt.subplots(2, 2, figsize=(16, 12))
fig.suptitle('Active Store Trading Trend Forecast (2026-2029)\n3-Year Projection with Upper/Lower Bounds', 
             fontsize=16, fontweight='bold', y=0.98)

colors = {'base': '#2196F3', 'upper': '#4CAF50', 'lower': '#FF5722', 'fill': '#BBDEFB'}

# Chart 1: Active Stores Trend
ax1 = axes[0, 0]
ax1.fill_between(month_labels, lower['active_stores'], upper['active_stores'], 
                  alpha=0.2, color=colors['fill'], label='Confidence Band')
ax1.plot(month_labels, base['active_stores'], color=colors['base'], linewidth=2.5, label='Base Case')
ax1.plot(month_labels, upper['active_stores'], color=colors['upper'], linewidth=1.5, 
         linestyle='--', label='Upper Bound')
ax1.plot(month_labels, lower['active_stores'], color=colors['lower'], linewidth=1.5, 
         linestyle='--', label='Lower Bound')
ax1.scatter(['2026-04'], [avg_active_stores], color='red', s=100, zorder=5, label='Current')
ax1.set_title('Daily Active Stores Forecast', fontsize=13, fontweight='bold')
ax1.set_ylabel('Active Stores', fontsize=11)
ax1.legend(fontsize=9)
ax1.tick_params(axis='x', rotation=45)
ax1.set_xticks(month_labels[::4])
ax1.yaxis.set_major_formatter(mticker.FuncFormatter(lambda x, p: f'{x/1000:.0f}K'))
ax1.grid(True, alpha=0.3)

# Chart 2: Monthly Transaction Amount
ax2 = axes[0, 1]
ax2.fill_between(month_labels, lower['monthly_amount'], upper['monthly_amount'], 
                  alpha=0.2, color=colors['fill'], label='Confidence Band')
ax2.plot(month_labels, base['monthly_amount'], color=colors['base'], linewidth=2.5, label='Base Case')
ax2.plot(month_labels, upper['monthly_amount'], color=colors['upper'], linewidth=1.5, 
         linestyle='--', label='Upper Bound')
ax2.plot(month_labels, lower['monthly_amount'], color=colors['lower'], linewidth=1.5, 
         linestyle='--', label='Lower Bound')
current_monthly_amount = avg_daily_amount * 30 / 1e8
ax2.scatter(['2026-04'], [current_monthly_amount], color='red', s=100, zorder=5, label='Current')
ax2.set_title('Monthly Transaction Volume Forecast (100M)', fontsize=13, fontweight='bold')
ax2.set_ylabel('Transaction Volume (Yi CNY)', fontsize=11)
ax2.legend(fontsize=9)
ax2.tick_params(axis='x', rotation=45)
ax2.set_xticks(month_labels[::4])
ax2.grid(True, alpha=0.3)

# Chart 3: Monthly New Store Addition Trend (Historical + Forecast)
ax3 = axes[1, 0]
hist_months = []
for y, stores in [(2024, new_stores_2024), (2025, new_stores_2025), (2026, new_stores_2026)]:
    for i, s in enumerate(stores):
        m = i + 1
        hist_months.append((f'{y}-{m:02d}', s))

hist_labels = [h[0] for h in hist_months]
hist_values = [h[1] for h in hist_months]

# Add forecast months
all_new_labels = hist_labels + month_labels
all_new_values = hist_values + [base_monthly_new] * 36
upper_new = hist_values + [base_monthly_new * 1.2] * 36
lower_new = hist_values + [base_monthly_new * 0.8] * 36

ax3.bar(hist_labels, hist_values, color='#90CAF9', alpha=0.8, label='Historical')
ax3.bar(month_labels, [base_monthly_new] * 36, color='#42A5F5', alpha=0.6, label='Forecast (Base)')
ax3.fill_between(month_labels, [base_monthly_new * 0.8] * 36, [base_monthly_new * 1.2] * 36,
                  alpha=0.15, color=colors['fill'], label='Forecast Range')
ax3.axhline(y=recent_monthly_avg, color='red', linestyle=':', alpha=0.7, label=f'Avg={recent_monthly_avg:.0f}')
ax3.set_title('Monthly New Store Onboarding (Historical + Forecast)', fontsize=13, fontweight='bold')
ax3.set_ylabel('New Stores', fontsize=11)
ax3.legend(fontsize=8)
ax3.tick_params(axis='x', rotation=45, labelsize=7)
ax3.set_xticks(list(range(0, len(all_new_labels), 3)))
ax3.set_xticklabels([all_new_labels[i] for i in range(0, len(all_new_labels), 3)], rotation=45)
ax3.grid(True, alpha=0.3)

# Chart 4: Activation Rate Trend
ax4 = axes[1, 1]
ax4.fill_between(month_labels, 
                  [r * 100 for r in lower['activation_rate']], 
                  [r * 100 for r in upper['activation_rate']], 
                  alpha=0.2, color=colors['fill'], label='Confidence Band')
ax4.plot(month_labels, [r * 100 for r in base['activation_rate']], 
         color=colors['base'], linewidth=2.5, label='Base Case')
ax4.plot(month_labels, [r * 100 for r in upper['activation_rate']], 
         color=colors['upper'], linewidth=1.5, linestyle='--', label='Upper Bound')
ax4.plot(month_labels, [r * 100 for r in lower['activation_rate']], 
         color=colors['lower'], linewidth=1.5, linestyle='--', label='Lower Bound')
ax4.scatter(['2026-04'], [activation_rate * 100], color='red', s=100, zorder=5, label='Current')
ax4.set_title('Store Activation Rate Trend', fontsize=13, fontweight='bold')
ax4.set_ylabel('Activation Rate (%)', fontsize=11)
ax4.legend(fontsize=9)
ax4.tick_params(axis='x', rotation=45)
ax4.set_xticks(month_labels[::4])
ax4.grid(True, alpha=0.3)

plt.tight_layout(rect=[0, 0, 1, 0.96])
output_path = '/Users/sm4645/work/claw-code/analysis_output/active_store_forecast_3yr.png'
os.makedirs(os.path.dirname(output_path), exist_ok=True)
plt.savefig(output_path, dpi=150, bbox_inches='tight', facecolor='white')
plt.close()
print(f"\nChart saved to: {output_path}")

# Print key forecast numbers
print("\n=== 3-Year Forecast Summary ===")
for year_idx, label in [(11, "End 2027"), (23, "End 2028"), (35, "End 2029")]:
    print(f"\n{label}:")
    print(f"  Active Stores: {lower['active_stores'][year_idx]:,.0f} - {base['active_stores'][year_idx]:,.0f} - {upper['active_stores'][year_idx]:,.0f}")
    print(f"  Monthly Amount: {lower['monthly_amount'][year_idx]:.1f} - {base['monthly_amount'][year_idx]:.1f} - {upper['monthly_amount'][year_idx]:.1f} (Yi)")
    print(f"  Activation Rate: {lower['activation_rate'][year_idx]:.1%} - {base['activation_rate'][year_idx]:.1%} - {upper['activation_rate'][year_idx]:.1%}")
