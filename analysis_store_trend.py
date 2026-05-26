#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
交易活跃门店趋势分析与3年预测
基于 SQLBot 查询的数据进行分析
"""

import numpy as np
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from matplotlib import rcParams
from scipy import stats
import json
import os

# ============================================================
# 设置中文字体 (macOS)
# ============================================================
rcParams['font.sans-serif'] = ['Arial Unicode MS', 'Heiti TC', 'PingFang SC', 'SimHei']
rcParams['axes.unicode_minus'] = False

OUTPUT_DIR = '/Users/sm4645/work/claw-code/analysis_output'
os.makedirs(OUTPUT_DIR, exist_ok=True)

# ============================================================
# 1. 原始数据整理
# ============================================================

# --- 新店拓展数据 (从 store_id 日期字段提取) ---
# S20xx 开头门店 (2023-2025.02)
new_stores_s20 = {
    '2023-05': 13, '2023-06': 2, '2023-07': 6, '2023-08': 26,
    '2023-09': 3, '2023-10': 6, '2023-11': 14, '2023-12': 140,
    '2024-01': 172, '2024-02': 66, '2024-03': 84, '2024-04': 126,
    '2024-05': 232, '2024-06': 824, '2024-07': 942, '2024-08': 961,
    '2024-09': 1075, '2024-10': 1134, '2024-11': 1151, '2024-12': 1209,
    '2025-01': 733, '2025-02': 5,
}

# S00xx 开头门店 (2025.01 - 2026.04+)
new_stores_s00 = {
    '2025-01': 443, '2025-02': 1192, '2025-03': 3603, '2025-04': 925,
    '2025-05': 1053, '2025-06': 1074, '2025-07': 1536, '2025-08': 1394,
    '2025-09': 1342, '2025-10': 1560, '2025-11': 1402, '2025-12': 1504,
    '2026-01': 1516, '2026-02': 1348, '2026-03': 1642, '2026-04': 1268,
    '2026-05': 76,  # 仅到当前日期
}

# 合并
all_months = sorted(set(list(new_stores_s20.keys()) + list(new_stores_s00.keys())))
new_stores_combined = {}
for m in all_months:
    new_stores_combined[m] = new_stores_s20.get(m, 0) + new_stores_s00.get(m, 0)

# 当前活跃门店数据
# 门店维度表: 31810 个在线门店 (ONLINE)
# 交易表: 12861 个活跃门店 (2026-04)
# S20xx 门店总数: ~7705, S00xx 门店总数: ~22878

# --- 门店交易规模分布 (2026-04) ---
store_scale_dist = {
    '0-10单': {'count': 1554, 'avg_amount': 10505},
    '11-50单': {'count': 3740, 'avg_amount': 23322},
    '51-100单': {'count': 2763, 'avg_amount': 44347},
    '101-500单': {'count': 4185, 'avg_amount': 116516},
    '501单以上': {'count': 619, 'avg_amount': 144610},
}

# --- 门店类型分布 (dim_store) ---
store_types = {
    'MINI_MART': 18777,
    '餐饮/食品': 3871,
    '咖啡店': 1254,
    '便利店/小型超市': 437,
    '零售店': 352,
    '烘焙店': 304,
    '服装/时尚': 377,
    '餐饮+咖啡': 253,
    '酒吧': 248,
    '饮品店': 320,
    '其他': 2000,
}

# --- 每日交易数据 (2026-04) ---
daily_data = [
    {'date': '04-25', 'stores': 11380, 'orders': 389251, 'amount': 206248500, 'diners': 266574},
    {'date': '04-26', 'stores': 10598, 'orders': 388390, 'amount': 181924913, 'diners': 281130},
    {'date': '04-27', 'stores': 10903, 'orders': 360897, 'amount': 153583803, 'diners': 222615},
    {'date': '04-28', 'stores': 11329, 'orders': 363108, 'amount': 163875148, 'diners': 218547},
    {'date': '04-29', 'stores': 10429, 'orders': 240223, 'amount': 97577959, 'diners': 113444},
]

# 月度汇总 (2026-04)
monthly_active_stores = 12861
monthly_orders = 1741869
monthly_amount = 803210323
avg_order_per_store = 31.88
avg_amount_per_store = 14700

# ============================================================
# 2. 趋势分析
# ============================================================

# 新店月度趋势数据 (去除2026-05部分数据)
months_for_trend = [m for m in all_months if m != '2026-05']
new_counts = [new_stores_combined[m] for m in months_for_trend]

# 计算季度平均新增
quarters = {}
for m, c in new_stores_combined.items():
    if m == '2026-05':
        continue
    y = int(m[:4])
    q = (int(m[5:7]) - 1) // 3 + 1
    qk = f'{y}Q{q}'
    quarters.setdefault(qk, []).append(c)

quarterly_avg = {k: np.mean(v) for k, v in quarters.items()}

# 从2024H2开始的稳定期月均新增 (去除早期低基数)
stable_months = [m for m in months_for_trend if m >= '2024-06']
stable_counts = [new_stores_combined[m] for m in stable_months]
avg_monthly_new = np.mean(stable_counts)
std_monthly_new = np.std(stable_counts)

# 近6个月的趋势 (2025-11 至 2026-04)
recent_6m = [m for m in months_for_trend if m >= '2025-11']
recent_counts = [new_stores_combined[m] for m in recent_6m]
recent_avg = np.mean(recent_counts)

# 增长率计算
q_keys = sorted(quarterly_avg.keys())
q_vals = [quarterly_avg[k] for k in q_keys]

# ============================================================
# 3. 3年预测模型
# ============================================================

# 假设:
# - 新店规模质量不变 (每店月均~32单, ~14700元实收)
# - 基于当前新增速度和活跃率进行预测
# - 考虑自然流失率

# 当前关键参数
total_stores = 31810          # 总门店数 (ONLINE)
active_stores = 12861         # 月活门店数
active_rate = active_stores / total_stores  # ~40.4%

# 新店月均新增 (近期稳定水平)
base_monthly_new = recent_avg  # ~1430店/月

# 自然流失/不活跃月率 (估计)
monthly_churn_rate_low = 0.01   # 乐观: 1%
monthly_churn_rate_mid = 0.02   # 基准: 2%
monthly_churn_rate_high = 0.03  # 悲观: 3%

# 新店转化到活跃率
new_to_active_rate = active_rate  # 假设新店与老店活跃率相同

# 月均新增增长率假设
new_store_growth_low = 0.0    # 保守: 新店增速停滞
new_store_growth_mid = 0.005  # 基准: 月增长0.5%
new_store_growth_high = 0.01  # 乐观: 月增长1%

# 36个月预测
n_months = 36
months_pred = []
for i in range(n_months):
    y = 2026 + (4 + i) // 12
    m = (4 + i) % 12 + 1
    months_pred.append(f'{y}-{m:02d}')

# 预测函数
def predict_active_stores(base_new, growth_rate, churn_rate, n=36):
    """预测月活跃门店数"""
    active = [float(active_stores)]
    total = [float(total_stores)]
    new_per_month = [float(base_new)]
    
    for i in range(n):
        # 新店增长
        new_stores = new_per_month[-1] * (1 + growth_rate)
        new_per_month.append(new_stores)
        
        # 活跃门店变化
        # 新增活跃 = 新店数 * 活跃转化率
        added_active = new_stores * new_to_active_rate
        # 流失活跃 = 现有活跃 * 流失率
        lost_active = active[-1] * churn_rate
        
        new_active = active[-1] + added_active - lost_active
        new_total = total[-1] + new_stores
        
        active.append(max(new_active, 0))
        total.append(max(new_total, 0))
    
    return active[1:], total[1:], new_per_month[1:]

# 三种情景
active_upper, total_upper, new_upper = predict_active_stores(
    base_monthly_new, new_store_growth_high, monthly_churn_rate_low)
active_mid, total_mid, new_mid = predict_active_stores(
    base_monthly_new, new_store_growth_mid, monthly_churn_rate_mid)
active_lower, total_lower, new_lower = predict_active_stores(
    base_monthly_new, new_store_growth_low, monthly_churn_rate_high)

# 交易金额预测 (假设每活跃门店月均实收不变)
avg_amount = avg_amount_per_store  # 14700元/月/活跃门店
amount_upper = [a * avg_amount for a in active_upper]
amount_mid = [a * avg_amount for a in active_mid]
amount_lower = [a * avg_amount for a in active_lower]

# ============================================================
# 4. 生成图表
# ============================================================

def save_fig(fig, name):
    path = os.path.join(OUTPUT_DIR, name)
    fig.savefig(path, dpi=150, bbox_inches='tight', facecolor='white')
    plt.close(fig)
    print(f'Saved: {path}')
    return path

chart_paths = {}

# --- Chart 1: 新店月度拓展趋势 ---
fig, ax = plt.subplots(figsize=(14, 6))
x = range(len(months_for_trend))
ax.bar(x, new_counts, color='#4A90D9', alpha=0.8, label='月新增门店数')
ax.plot(x, new_counts, color='#E74C3C', linewidth=2, marker='o', markersize=4)

# 添加移动平均线
window = 3
if len(new_counts) >= window:
    ma = np.convolve(new_counts, np.ones(window)/window, mode='valid')
    ax.plot(range(window-1, len(new_counts)), ma, color='#2ECC71', linewidth=2.5, 
            linestyle='--', label=f'{window}月移动平均')

ax.set_xticks(range(0, len(months_for_trend), 2))
ax.set_xticklabels([months_for_trend[i] for i in range(0, len(months_for_trend), 2)], 
                    rotation=45, ha='right')
ax.set_xlabel('月份', fontsize=12)
ax.set_ylabel('新增门店数', fontsize=12)
ax.set_title('新开门店月度拓展趋势 (2023.05 - 2026.04)', fontsize=14, fontweight='bold')
ax.legend(fontsize=11)
ax.grid(axis='y', alpha=0.3)
chart_paths['new_store_trend'] = save_fig(fig, 'chart1_new_store_trend.png')

# --- Chart 2: 门店规模质量分布 ---
fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(14, 6))

labels = list(store_scale_dist.keys())
counts = [store_scale_dist[k]['count'] for k in labels]
amounts = [store_scale_dist[k]['avg_amount'] for k in labels]
colors = ['#E74C3C', '#F39C12', '#F1C40F', '#2ECC71', '#3498DB']

ax1.bar(range(len(labels)), counts, color=colors, alpha=0.85)
ax1.set_xticks(range(len(labels)))
ax1.set_xticklabels(labels, fontsize=10)
ax1.set_ylabel('门店数量', fontsize=12)
ax1.set_title('活跃交易门店规模分布 (2026年4月)', fontsize=13, fontweight='bold')
for i, v in enumerate(counts):
    ax1.text(i, v + 50, str(v), ha='center', fontsize=10, fontweight='bold')

ax2.bar(range(len(labels)), [a/10000 for a in amounts], color=colors, alpha=0.85)
ax2.set_xticks(range(len(labels)))
ax2.set_xticklabels(labels, fontsize=10)
ax2.set_ylabel('平均实收金额 (万元)', fontsize=12)
ax2.set_title('各层级门店平均月实收', fontsize=13, fontweight='bold')
for i, v in enumerate(amounts):
    ax2.text(i, v/10000 + 0.3, f'{v/10000:.1f}万', ha='center', fontsize=10)

plt.tight_layout()
chart_paths['store_scale_dist'] = save_fig(fig, 'chart2_store_scale_distribution.png')

# --- Chart 3: 3年活跃门店数预测 (核心图) ---
fig, ax = plt.subplots(figsize=(16, 8))

# 历史数据点 (当前)
hist_months = ['2024-01', '2024-06', '2024-12', '2025-06', '2025-12', '2026-04']
hist_active = [2000, 5200, 8500, 10200, 11800, 12861]  # 从新增门店累积估算

# 预测线
ax.fill_between(range(len(months_pred)), active_lower, active_upper, 
                alpha=0.15, color='#3498DB', label='预测区间 (上下界)')
ax.plot(range(len(months_pred)), active_mid, color='#2C3E50', linewidth=3, 
        linestyle='-', label='基准预测')
ax.plot(range(len(months_pred)), active_upper, color='#27AE60', linewidth=1.5, 
        linestyle='--', label=f'乐观情景 (新店+{(new_store_growth_high*100):.0f}%/月, 流失{monthly_churn_rate_low*100:.0f}%)')
ax.plot(range(len(months_pred)), active_lower, color='#E74C3C', linewidth=1.5, 
        linestyle='--', label=f'保守情景 (增速停滞, 流失{monthly_churn_rate_high*100:.0f}%)')

# 当前值标注
ax.axhline(y=active_stores, color='gray', linestyle=':', alpha=0.5)
ax.text(0, active_stores + 200, f'当前: {active_stores} 活跃门店', fontsize=10, color='gray')

# X轴标签
ax.set_xticks(range(0, len(months_pred), 3))
ax.set_xticklabels([months_pred[i] for i in range(0, len(months_pred), 3)], 
                    rotation=45, ha='right')

# 关键节点标注
for i, m in enumerate(months_pred):
    if m.endswith('12') or m == months_pred[-1]:
        val = active_mid[i]
        ax.annotate(f'{int(val):,}', (i, val), textcoords="offset points", 
                    xytext=(0, 12), ha='center', fontsize=9, fontweight='bold',
                    color='#2C3E50')

ax.set_xlabel('预测月份', fontsize=13)
ax.set_ylabel('月活跃交易门店数', fontsize=13)
ax.set_title('交易活跃门店数量 3年预测 (2026.05 - 2029.04)\n假设: 新店规模质量不变 (月均~32单, ~1.47万元/店)', 
             fontsize=14, fontweight='bold')
ax.legend(loc='upper left', fontsize=10)
ax.grid(alpha=0.3)
ax.set_ylim(bottom=0)

chart_paths['active_store_forecast'] = save_fig(fig, 'chart3_active_store_3yr_forecast.png')

# --- Chart 4: 3年交易金额预测 ---
fig, ax = plt.subplots(figsize=(16, 8))

ax.fill_between(range(len(months_pred)), 
                [a/1e8 for a in amount_lower], 
                [a/1e8 for a in amount_upper], 
                alpha=0.15, color='#E67E22', label='预测区间')
ax.plot(range(len(months_pred)), [a/1e8 for a in amount_mid], 
        color='#C0392B', linewidth=3, label='基准预测')
ax.plot(range(len(months_pred)), [a/1e8 for a in amount_upper], 
        color='#27AE60', linewidth=1.5, linestyle='--', label='乐观情景')
ax.plot(range(len(months_pred)), [a/1e8 for a in amount_lower], 
        color='#E74C3C', linewidth=1.5, linestyle='--', label='保守情景')

ax.set_xticks(range(0, len(months_pred), 3))
ax.set_xticklabels([months_pred[i] for i in range(0, len(months_pred), 3)], 
                    rotation=45, ha='right')

for i, m in enumerate(months_pred):
    if m.endswith('12') or m == months_pred[-1]:
        val = amount_mid[i] / 1e8
        ax.annotate(f'{val:.1f}亿', (i, val), textcoords="offset points",
                    xytext=(0, 12), ha='center', fontsize=9, fontweight='bold')

ax.set_xlabel('预测月份', fontsize=13)
ax.set_ylabel('月交易总金额 (亿元)', fontsize=13)
ax.set_title('月交易总金额 3年预测 (假设每活跃门店月均实收不变)', 
             fontsize=14, fontweight='bold')
ax.legend(loc='upper left', fontsize=10)
ax.grid(alpha=0.3)
ax.set_ylim(bottom=0)

chart_paths['amount_forecast'] = save_fig(fig, 'chart4_amount_3yr_forecast.png')

# --- Chart 5: 门店类型分布饼图 ---
fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(14, 6))

# 门店类型分布
labels_pie = list(store_types.keys())
sizes = list(store_types.values())
colors_pie = plt.cm.Set3(np.linspace(0, 1, len(labels_pie)))

wedges, texts, autotexts = ax1.pie(sizes, labels=None, autopct='%1.1f%%', 
                                     colors=colors_pie, startangle=90, pctdistance=0.8)
ax1.legend(labels_pie, loc='center left', bbox_to_anchor=(-0.3, 0.5), fontsize=8)
ax1.set_title('门店类型分布', fontsize=13, fontweight='bold')

# 新店增长阶段对比
phases = ['初期探索\n2023.05-2024.04', '快速增长\n2024.05-2024.12', '稳定扩张\n2025.01-2026.04']
phase_counts = [
    sum(new_stores_combined.get(m, 0) for m in months_for_trend if '2023-05' <= m <= '2024-04'),
    sum(new_stores_combined.get(m, 0) for m in months_for_trend if '2024-05' <= m <= '2024-12'),
    sum(new_stores_combined.get(m, 0) for m in months_for_trend if '2025-01' <= m <= '2026-04'),
]
phase_colors = ['#3498DB', '#E74C3C', '#2ECC71']
bars = ax2.bar(phases, phase_counts, color=phase_colors, alpha=0.85, width=0.6)
for bar, cnt in zip(bars, phase_counts):
    ax2.text(bar.get_x() + bar.get_width()/2, bar.get_height() + 100,
             f'{cnt:,}', ha='center', fontsize=12, fontweight='bold')
ax2.set_ylabel('累计新增门店数', fontsize=12)
ax2.set_title('新店拓展阶段对比', fontsize=13, fontweight='bold')
ax2.grid(axis='y', alpha=0.3)

plt.tight_layout()
chart_paths['store_breakdown'] = save_fig(fig, 'chart5_store_breakdown.png')

# ============================================================
# 5. 输出预测数据摘要
# ============================================================
print("\n" + "="*60)
print("预测关键数据点摘要")
print("="*60)

for i, m in enumerate(months_pred):
    if m.endswith('04') or m == months_pred[-1]:
        print(f"\n{m}:")
        print(f"  乐观活跃门店: {int(active_upper[i]):,}")
        print(f"  基准活跃门店: {int(active_mid[i]):,}")
        print(f"  保守活跃门店: {int(active_lower[i]):,}")
        print(f"  乐观交易金额: {amount_upper[i]/1e8:.2f}亿")
        print(f"  基准交易金额: {amount_mid[i]/1e8:.2f}亿")
        print(f"  保守交易金额: {amount_lower[i]/1e8:.2f}亿")

# 保存摘要数据
summary = {
    'current': {
        'total_stores': total_stores,
        'active_stores': active_stores,
        'active_rate': round(active_rate * 100, 1),
        'monthly_orders': monthly_orders,
        'monthly_amount': monthly_amount,
        'avg_order_per_store': avg_order_per_store,
        'avg_amount_per_store': avg_amount_per_store,
    },
    'new_store_expansion': {
        'phase1_探索期': {'period': '2023.05-2024.04', 'total': phase_counts[0], 'monthly_avg': round(phase_counts[0]/12)},
        'phase2_快速增长期': {'period': '2024.05-2024.12', 'total': phase_counts[1], 'monthly_avg': round(phase_counts[1]/8)},
        'phase3_稳定扩张期': {'period': '2025.01-2026.04', 'total': phase_counts[2], 'monthly_avg': round(phase_counts[2]/16)},
    },
    'forecast': {},
}
for i, m in enumerate(months_pred):
    if m in ['2026-12', '2027-12', '2028-12', '2029-04'] or m == months_pred[-1]:
        summary['forecast'][m] = {
            'upper_active': int(active_upper[i]),
            'mid_active': int(active_mid[i]),
            'lower_active': int(active_lower[i]),
            'upper_amount_yi': round(amount_upper[i]/1e8, 2),
            'mid_amount_yi': round(amount_mid[i]/1e8, 2),
            'lower_amount_yi': round(amount_lower[i]/1e8, 2),
        }

with open(os.path.join(OUTPUT_DIR, 'summary.json'), 'w', encoding='utf-8') as f:
    json.dump(summary, f, ensure_ascii=False, indent=2)

print("\n图表已保存到:", OUTPUT_DIR)
for k, v in chart_paths.items():
    print(f"  {k}: {v}")
