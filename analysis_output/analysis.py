#!/usr/bin/env python3
"""Store scale and 3-year forecast analysis"""
import json, os, sys
import numpy as np

# === Try matplotlib with different backends ===
try:
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    import matplotlib.ticker as mticker
    HAS_MPL = True
except Exception:
    HAS_MPL = False

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
org_count = 89
total_orders_4d = 1352618

# === DERIVED METRICS ===
daily_avg_orders = np.mean([d['orders'] for d in daily_data.values()])
daily_avg_revenue = np.mean([d['revenue'] for d in daily_data.values()])
daily_avg_stores = np.mean([d['stores'] for d in daily_data.values()])
orders_per_store = daily_avg_orders / daily_avg_stores  # ~31.3
revenue_per_store = daily_avg_revenue / daily_avg_stores
aov = daily_avg_revenue / daily_avg_orders
stores_pending = total_stores_dim - stores_with_trade

# Monthly
monthly_orders = daily_avg_orders * 30
monthly_revenue = daily_avg_revenue * 30

print(f"当前日均订单: {daily_avg_orders:,.0f}")
print(f"当前日均营收: {daily_avg_revenue:,.0f}")
print(f"当前日均活跃门店: {daily_avg_stores:.0f}")
print(f"单店日均订单: {orders_per_store:.1f}")
print(f"客单价: {aov:.0f}")
print(f"月订单量(估): {monthly_orders:,.0f}")
print(f"月营收(估): {monthly_revenue:,.0f}")

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

print(f"\n=== 3年预测结果 ===")
for scenario, stores in [("保守(低)", low_stores), ("基准(中)", mid_stores), ("乐观(高)", high_stores)]:
    m36_stores = stores[36]
    m36_orders_monthly = m36_stores * orders_per_store * 30
    m36_revenue_monthly = m36_stores * revenue_per_store * 30
    print(f"\n{scenario}:")
    print(f"  Y1 (M12): {stores[12]:,.0f} 门店, 月订单 {stores[12]*orders_per_store*30/1e6:.1f}M, 月营收 {stores[12]*revenue_per_store*30/1e9:.2f}B")
    print(f"  Y2 (M24): {stores[24]:,.0f} 门店, 月订单 {stores[24]*orders_per_store*30/1e6:.1f}M, 月营收 {stores[24]*revenue_per_store*30/1e9:.2f}B")
    print(f"  Y3 (M36): {m36_stores:,.0f} 门店, 月订单 {m36_orders_monthly/1e6:.1f}M, 月营收 {m36_revenue_monthly/1e9:.2f}B")

# Print table for report
print(f"\n=== 预测数据表 ===")
print(f"指标,M0,M6,M12,M18,M24,M30,M36")
for scenario, data in [("保守", low_stores), ("基准", mid_stores), ("乐观", high_stores)]:
    vals = ",".join([f"{data[i]:,.0f}" for i in [0,6,12,18,24,30,36]])
    print(f"{scenario}门店数,{vals}")
    order_vals = ",".join([f"{data[i]*orders_per_store*30/1e6:.1f}" for i in [0,6,12,18,24,30,36]])
    print(f"{scenario}月订单量(M),{order_vals}")
    rev_vals = ",".join([f"{data[i]*revenue_per_store*30/1e9:.2f}" for i in [0,6,12,18,24,30,36]])
    print(f"{scenario}月营收(B),{rev_vals}")

# === CHARTS ===
if HAS_MPL:
    # Chart 1: Daily Snapshot
    fig, axes = plt.subplots(1, 3, figsize=(16, 5))
    days = list(daily_data.keys())
    day_labels = ['4/26\n(周日)', '4/27\n(周一)', '4/28\n(周二)', '4/29\n(周三)']
    colors = ['#2196F3', '#4CAF50', '#FF9800', '#9C27B0']

    orders_vals = [d['orders'] for d in daily_data.values()]
    axes[0].bar(day_labels, orders_vals, color=colors, alpha=0.85)
    axes[0].set_title('日订单量', fontsize=14, fontweight='bold')
    axes[0].set_ylabel('订单数')
    axes[0].yaxis.set_major_formatter(mticker.FuncFormatter(lambda x, _: f'{x/1000:.0f}k'))
    for i, v in enumerate(orders_vals):
        axes[0].text(i, v+5000, f'{v/1000:.0f}k', ha='center', fontsize=9)

    stores_vals = [d['stores'] for d in daily_data.values()]
    axes[1].bar(day_labels, stores_vals, color=colors, alpha=0.85)
    axes[1].set_title('日活跃门店数', fontsize=14, fontweight='bold')
    for i, v in enumerate(stores_vals):
        axes[1].text(i, v+100, f'{v:,}', ha='center', fontsize=9)

    rev_vals = [d['revenue']/1e6 for d in daily_data.values()]
    axes[2].bar(day_labels, rev_vals, color=colors, alpha=0.85)
    axes[2].set_title('日营收 (百万)', fontsize=14, fontweight='bold')
    for i, v in enumerate(rev_vals):
        axes[2].text(i, v+1, f'{v:.0f}M', ha='center', fontsize=9)

    plt.tight_layout()
    plt.savefig('analysis_output/chart1_daily_snapshot.png', dpi=150, bbox_inches='tight')
    plt.close()
    print("✅ Chart 1 saved")

    # Chart 2: 3-Year Store Forecast
    fig, ax = plt.subplots(figsize=(14, 7))
    ax.fill_between(months, low_stores, high_stores, alpha=0.12, color='#4CAF50', label='预测区间 (保守↔乐观)')
    ax.plot(months, mid_stores, 'b-', linewidth=3, label=f'基准预测 (15%年增长)', marker='o', markersize=3, markevery=3)
    ax.plot(months, high_stores, 'r--', linewidth=2, label=f'乐观预测 (25%年增长)', marker='s', markersize=3, markevery=3)
    ax.plot(months, low_stores, 'g--', linewidth=2, label=f'保守预测 (5%年增长)', marker='^', markersize=3, markevery=3)

    ax.axhline(y=total_stores_dim, color='orange', linestyle=':', linewidth=1.5, label=f'已注册门店数 ({total_stores_dim:,})')
    ax.axhline(y=stores_with_trade, color='gray', linestyle='-.', linewidth=1, label=f'当前活跃 (M0: {stores_with_trade:,})')

    ax.set_xlabel('月份 (M0=2026.05)', fontsize=12)
    ax.set_ylabel('活跃门店数', fontsize=12)
    ax.set_title('活跃门店规模 3年预测', fontsize=16, fontweight='bold')
    ax.legend(loc='upper left', fontsize=9)
    ax.yaxis.set_major_formatter(mticker.FuncFormatter(lambda x, _: f'{x/1000:.0f}k'))
    ax.set_xlim(-0.5, 37)
    ax.grid(True, alpha=0.3)
    ax.set_xticks(range(0, 37, 3))

    # Key annotations
    for v, s, c in [(high_stores[12], '高Y1', 'red'), (mid_stores[36], '基准Y3', 'blue'), (low_stores[36], '保守Y3', 'green')]:
        ax.annotate(f'{v:,.0f}', xy=(months[list(s.startswith('高') and range(13) or range(37)).index(v) if False else (12 if s=='高Y1' else 36)], v),
                    xytext=(-30 if s!='高Y1' else 10, 15), textcoords='offset points',
                    fontsize=8, ha='center', color=c)

    plt.tight_layout()
    plt.savefig('analysis_output/chart2_store_forecast.png', dpi=150, bbox_inches='tight')
    plt.close()
    print("✅ Chart 2 saved")

    # Chart 3: Revenue Forecast
    fig, ax = plt.subplots(figsize=(14, 7))
    low_rev = [s * revenue_per_store * 30 / 1e9 for s in low_stores]
    mid_rev = [s * revenue_per_store * 30 / 1e9 for s in mid_stores]
    high_rev = [s * revenue_per_store * 30 / 1e9 for s in high_stores]

    ax.fill_between(months, low_rev, high_rev, alpha=0.12, color='#FF9800', label='预测区间')
    ax.plot(months, mid_rev, 'b-', linewidth=3, label='基准预测')
    ax.plot(months, high_rev, 'r--', linewidth=2, label='乐观预测')
    ax.plot(months, low_rev, 'g--', linewidth=2, label='保守预测')

    ax.set_xlabel('月份', fontsize=12)
    ax.set_ylabel('月营收 (十亿)', fontsize=12)
    ax.set_title('月营收 3年预测 (假设单店质量不变)', fontsize=16, fontweight='bold')
    ax.legend(loc='upper left', fontsize=10)
    ax.set_xlim(-0.5, 37)
    ax.grid(True, alpha=0.3)
    ax.set_xticks(range(0, 37, 3))
    ax.yaxis.set_major_formatter(mticker.FuncFormatter(lambda x, _: f'{x:.1f}B'))

    plt.tight_layout()
    plt.savefig('analysis_output/chart3_revenue_forecast.png', dpi=150, bbox_inches='tight')
    plt.close()
    print("✅ Chart 3 saved")

    # Chart 4: Hourly Pattern
    fig, ax = plt.subplots(figsize=(14, 5))
    hours = list(range(24))
    orders_by_hour = [hourly_orders.get(h, 0) for h in hours]
    colors_bar = ['#FF5722' if h in [12,13,14,18,19,20] else '#607D8B' for h in hours]
    ax.bar(hours, orders_by_hour, color=colors_bar, alpha=0.85, width=0.8)
    ax.set_xlabel('小时', fontsize=12)
    ax.set_ylabel('订单数', fontsize=12)
    ax.set_title('24小时交易分布 (4天汇总)', fontsize=14, fontweight='bold')
    ax.set_xticks(range(0, 24, 2))
    ax.yaxis.set_major_formatter(mticker.FuncFormatter(lambda x, _: f'{x/1000:.0f}k'))

    # Add lunch/dinner annotations
    ax.annotate('午餐高峰\n12-15点', xy=(13.5, 155692), xytext=(14, 160000),
                fontsize=10, ha='center', color='#D84315',
                arrowprops=dict(arrowstyle='->', color='#D84315', lw=1.5))
    ax.annotate('晚餐高峰\n18-20点', xy=(19, 118149), xytext=(20, 130000),
                fontsize=10, ha='center', color='#D84315',
                arrowprops=dict(arrowstyle='->', color='#D84315', lw=1.5))

    plt.tight_layout()
    plt.savefig('analysis_output/chart4_hourly_pattern.png', dpi=150, bbox_inches='tight')
    plt.close()
    print("✅ Chart 4 saved")

    # Chart 5: Store Activity Distribution
    fig, ax = plt.subplots(figsize=(10, 5))
    days_labels = list(store_active_days.keys())
    counts = [store_active_days[d] for d in days_labels]
    pie_colors = ['#FF5722', '#FF9800', '#FFC107', '#4CAF50']
    explode = (0, 0, 0, 0.05)
    wedges, texts, autotexts = ax.pie(counts, explode=explode, labels=[f'{d}天' for d in days_labels],
                                        autopct='%1.1f%%', colors=pie_colors, startangle=90,
                                        textprops={'fontsize': 11})
    ax.set_title(f'门店活跃天数分布 (总计 {stores_with_trade:,} 店)', fontsize=14, fontweight='bold')

    plt.tight_layout()
    plt.savefig('analysis_output/chart5_activity_distribution.png', dpi=150, bbox_inches='tight')
    plt.close()
    print("✅ Chart 5 saved")

    # Chart 6: Payment Distribution
    fig, ax = plt.subplots(figsize=(10, 6))
    pay_labels = list(payment.keys())
    pay_amounts = [payment[k]['amount']/1e6 for k in pay_labels]
    pay_colors = ['#4CAF50', '#2196F3', '#FF9800', '#9C27B0']
    bars = ax.bar(pay_labels, pay_amounts, color=pay_colors, alpha=0.85)
    ax.set_ylabel('金额 (百万)', fontsize=12)
    ax.set_title('支付方式金额分布', fontsize=14, fontweight='bold')
    total_pay = sum(pay_amounts)
    for bar, amt in zip(bars, pay_amounts):
        ax.text(bar.get_x() + bar.get_width()/2, bar.get_height() + 1,
                f'{amt:.0f}M\n({amt/total_pay*100:.1f}%)',
                ha='center', fontsize=10)
    plt.tight_layout()
    plt.savefig('analysis_output/chart6_payment.png', dpi=150, bbox_inches='tight')
    plt.close()
    print("✅ Chart 6 saved")

else:
    print("⚠️ Matplotlib不可用，跳过图表生成")

# === FORECAST CSV ===
with open('analysis_output/forecast_data.csv', 'w') as f:
    f.write("month,scenario,stores,monthly_orders,monthly_revenue\n")
    for scenario, data in [("low", low_stores), ("mid", mid_stores), ("high", high_stores)]:
        for i, s in enumerate(data):
            f.write(f"{i},{scenario},{s:.0f},{s*orders_per_store*30:.0f},{s*revenue_per_store*30:.0f}\n")

print("✅ Forecast CSV saved")
print("\nDone!")
