#!/usr/bin/env python3
"""
A股每日复盘生成器 - 全自动版
"""

import akshare as ak
import pandas as pd
from datetime import datetime
import warnings
import datetime as dt
warnings.filterwarnings('ignore')

# ============ 配置 =============
DATE = "20260310"
OUTPUT_PATH = f"/Users/huangjiahao/.openfang/workspaces/assistant/output/{DATE}_full.md"

# ============ 工具函数 ============
def format_num(x):
    if pd.isna(x):
        return "—"
    if x >= 1e8:
        return f"{x/1e8:.2f}亿"
    elif x >= 1e4:
        return f"{x/1e4:.2f}万"
    else:
        return str(x)

def get_time_str(t):
    if pd.isna(t):
        return "—"
    t = str(t).zfill(5)
    return f"{t[:2]}:{t[2:]}"

def get_10cm_or_20cm(pct):
    if pd.isna(pct):
        return "—"
    if pct >= 19.5:
        return "20cm"
    else:
        return "10cm"

# ============ 主逻辑 ============
print(f"📊 开始生成 {DATE} 复盘...")
print("=" * 50)

# 1. 获取涨跌比
print("📈 获取涨跌比...")
board_df = ak.stock_board_industry_name_em()
上涨 = board_df['上涨家数'].sum()
下跌 = board_df['下跌家数'].sum()
涨跌比 = f"{上涨}:{下跌} ({上涨/下跌:.2f}:1)" if 下跌 > 0 else "N/A"
print(f"   涨跌比: {涨跌比}")

# 2. 获取涨停池
print("📈 获取涨停池...")
zt_df = ak.stock_zt_pool_em(date=DATE)
zt_count = len(zt_df)
zt_df['炸板次数_num'] = pd.to_numeric(zt_df['炸板次数'], errors='coerce').fillna(0).astype(int)
zt_封死 = len(zt_df[zt_df['炸板次数_num'] == 0])
封板率 = zt_封死 / zt_count * 100 if zt_count > 0 else 0
print(f"   涨停: {zt_count}只 | 封板率: {封板率:.1f}%")

# 3. 获取炸板股池（冲高未涨停 + 炸板）
print("📈 获取炸板股池...")
zbgc_df = ak.stock_zt_pool_zbgc_em(date=DATE)
zbgc_count = len(zbgc_df)

# 冲高未涨停: 7% < 涨幅 < 9.5%
chonggao = zbgc_df[(zbgc_df['涨跌幅'] > 7) & (zbgc_df['涨跌幅'] < 9.5)].copy()
chonggao_count = len(chonggao)

# 真正的炸板: 收盘涨幅 > 5% 但没涨停
zhaban = zbgc_df[(zbgc_df['涨跌幅'] > 5) & (zbgc_df['涨跌幅'] < 9.5)].copy()
zhaban_count = len(zhaban)

# 跌停：从炸板股里找跌幅 > -9.5%的
diting = zbgc_df[zbgc_df['涨跌幅'] < -9].copy()
diting_count = len(diting)
print(f"   炸板股: {zbgc_count}只 | 冲高未涨停: {chonggao_count}只 | 跌停: {diting_count}只")

# 4. 昨日涨停今日表现
print("📈 获取昨日涨停表现...")
prev_zt = ak.stock_zt_pool_previous_em(date=DATE)
prev_zt_count = len(prev_zt)
prev_zt_继续涨停 = len(prev_zt[prev_zt['涨跌幅'] > 9])
prev_zt_跌停 = len(prev_zt[prev_zt['涨跌幅'] < -9])
print(f"   昨日涨停: {prev_zt_count}只 | 继续涨停: {prev_zt_继续涨停}只 | 跌停: {prev_zt_跌停}只")

# 5. 获取指数数据（动态计算昨天日期）
print("📈 获取指数数据...")
prev_date = dt.datetime.strptime(DATE, "%Y%m%d") - dt.timedelta(days=1)
prev_date_str = prev_date.strftime("%Y%m%d")

sh_df = ak.index_zh_a_hist(symbol="000001", period="daily", start_date=prev_date_str, end_date=DATE)
sz_df = ak.index_zh_a_hist(symbol="399001", period="daily", start_date=prev_date_str, end_date=DATE)
cyb_df = ak.index_zh_a_hist(symbol="399006", period="daily", start_date=prev_date_str, end_date=DATE)

sh = sh_df.iloc[-1]
sz = sz_df.iloc[-1]
cyb = cyb_df.iloc[-1]

sh_pct = sh['涨跌幅']
sz_pct = sz['涨跌幅']
cyb_pct = cyb['涨跌幅']

# 6. 处理涨停股数据
zt_df['封单占比'] = (zt_df['封板资金'] / zt_df['流通市值'] * 100).round(2)
zt_df['10cm/20cm'] = zt_df['涨跌幅'].apply(get_10cm_or_20cm)
zt_df['涨停时间_str'] = zt_df['首次封板时间'].apply(get_time_str)
zt_df['连板_str'] = zt_df['连板数'].apply(lambda x: f"{int(x)}连板" if x > 0 else "首板")

zt_sorted = zt_df.sort_values('首次封板时间').reset_index(drop=True)
zt_sorted['is_yizi'] = zt_sorted['首次封板时间'].astype(str).str.startswith('0925')

yizi = zt_sorted[zt_sorted['is_yizi'] == True].copy()
zaopan = zt_sorted[(zt_sorted['is_yizi'] == False) & (zt_sorted['首次封板时间'].astype(str) <= '113000')].copy()
wuhou = zt_sorted[(zt_sorted['首次封板时间'].astype(str) > '113000') & (zt_sorted['首次封板时间'].astype(str) <= '143000')].copy()

# 7. 处理冲高未涨停
chonggao['封单占比'] = (chonggao['成交额'] / chonggao['流通市值'] * 100).round(2)
chonggao = chonggao.sort_values('涨跌幅', ascending=False).reset_index(drop=True)

# 8. 找出板块集中度
industry_count = zt_df['所属行业'].value_counts().head(10)

# ============ 生成Markdown ============
md = f"""# {DATE[:4]}年{DATE[4:6]}月{DATE[6:]}日 完整复盘

> 数据来源：akshare | 生成时间：{datetime.now().strftime('%H:%M')}

---

### 一、基础数据

| 项目 | 数据 |
|------|------|
| 沪指 | {sh['收盘']:.2f}，{sh_pct:+.2f}% |
| 深成指 | {sz['收盘']:.2f}，{sz_pct:+.2f}% |
| 创业板 | {cyb['收盘']:.2f}，{cyb_pct:+.2f}% |
| 涨跌比 | {涨跌比} |
| 涨停 | {zt_count}只 |
| 跌停 | {diting_count}只 |
| 封板率 | {封板率:.2f}% |
| 昨日涨停表现 | {prev_zt_继续涨停}只继续涨停，{prev_zt_跌停}只跌停 |

---

### 二、涨停股详细清单（按涨停时间排序）

#### 一字板（9:25）

| 股票 | 代码 | 涨停时间 | 涨停 | 封单金额 | 封单占比 | 成交额 | 换手 | 连板 | 代表板块 |
|------|------|----------|------|----------|----------|--------|------|------|----------|
"""

for _, row in yizi.iterrows():
    md += f"| {row['名称']} | {row['代码']} | {row['涨停时间_str']} | {row['10cm/20cm']} | {format_num(row['封板资金'])} | {row['封单占比']:.2f}% | {format_num(row['成交额'])} | {row['换手率']:.2f}% | {row['连板_str']} | {row['所属行业']} |\n"

md += """
#### 早盘快速板（9:30-11:30）

| 股票 | 代码 | 涨停时间 | 涨停 | 封单金额 | 封单占比 | 成交额 | 换手 | 连板 | 代表板块 |
|------|------|----------|------|----------|----------|--------|------|------|----------|
"""

for _, row in zaopan.iterrows():
    md += f"| {row['名称']} | {row['代码']} | {row['涨停时间_str']} | {row['10cm/20cm']} | {format_num(row['封板资金'])} | {row['封单占比']:.2f}% | {format_num(row['成交额'])} | {row['换手率']:.2f}% | {row['连板_str']} | {row['所属行业']} |\n"

md += """
#### 午后首板（13:00-14:30）

| 股票 | 代码 | 涨停时间 | 涨停 | 封单金额 | 封单占比 | 成交额 | 换手 | 连板 | 代表板块 |
|------|------|----------|------|----------|----------|--------|------|------|----------|
"""

for _, row in wuhou.iterrows():
    md += f"| {row['名称']} | {row['代码']} | {row['涨停时间_str']} | {row['10cm/20cm']} | {format_num(row['封板资金'])} | {row['封单占比']:.2f}% | {format_num(row['成交额'])} | {row['换手率']:.2f}% | {row['连板_str']} | {row['所属行业']} |\n"

md += f"""
#### 冲高未涨停（7%-9%）

| 股票 | 代码 | 收盘涨幅 | 成交额 | 换手率 | 代表板块 |
|------|------|----------|--------|--------|----------|
"""

for _, row in chonggao.iterrows():
    md += f"| {row['名称']} | {row['代码']} | {row['涨跌幅']:.2f}% | {format_num(row['成交额'])} | {row['换手率']:.2f}% | {row['所属行业']} |\n"

# 今日跌停
md += f"""
#### 今日跌停

| 股票 | 代码 | 收盘跌幅 | 成交额 | 换手率 | 代表板块 |
|------|------|----------|--------|--------|----------|
"""

if len(diting) > 0:
    for _, row in diting.iterrows():
        md += f"| {row['名称']} | {row['代码']} | {row['涨跌幅']:.2f}% | {format_num(row['成交额'])} | {row['换手率']:.2f}% | {row['所属行业']} |\n"
else:
    md += "| — | — | — | — | — | — |\n"

# 板块集中度
md += f"""
---

### 三、涨停板块分布

"""

for industry, cnt in industry_count.items():
    md += f"- **{industry}**: {cnt}只\n"

# 全天走势复盘
md += f"""
---

### 四、全天走势复盘

"""

# 简单分析：按涨停时间分布
md += f"""**早盘（9:30-10:00）**: {len(zaopan[zaopan['首次封板时间'].astype(str) <= '100000'])}只涨停，主要集中在{zaopan[zaopan['首次封板时间'].astype(str) <= '100000']['所属行业'].value_counts().head(2).index.tolist() if len(zaopan) > 0 else '无'}

**上午（10:00-11:30）**: {len(zaopan[zaopan['首次封板时间'].astype(str) > '100000'])}只涨停

**午后（13:00-14:30）**: {len(wuhou)}只涨停

"""

# 反转龙头
top_by_money = zt_df.nlargest(3, '封板资金')
md += f"""---

### 五、龙头拆解

| 股票 | 代码 | 封单金额 | 封单占比 | 成交额 | 换手 | 连板 | 涨停原因 |
|------|------|----------|----------|--------|------|------|----------|
"""

for _, row in top_by_money.iterrows():
    md += f"| {row['名称']} | {row['代码']} | {format_num(row['封板资金'])} | {row['封单占比']:.2f}% | {format_num(row['成交额'])} | {row['换手率']:.2f}% | {row['连板_str']} | {row['所属行业']} |\n"

# 亏钱效应
# 找昨天涨停今天大跌的
prev_zt_die = prev_zt[prev_zt['涨跌幅'] < -5]
md += f"""
---

### 六、亏钱效应

**昨日涨停今日大跌（>5%）**: {len(prev_zt_die)}只
"""

if len(prev_zt_die) > 0:
    for _, row in prev_zt_die.head(5).iterrows():
        md += f"- {row['名称']} {row['代码']}: {row['涨跌幅']:.2f}%\n"

md += f"""
**冲高未涨停**: {chonggao_count}只

"""

# 总结
md += f"""---

### 七、操作总结

| 指标 | 数据 |
|------|------|
| 涨跌比 | {涨跌比} |
| 涨停 | {zt_count}只 |
| 跌停 | {diting_count}只 |
| 封板率 | {封板率:.1f}% |
| 炸板股 | {zhaban_count}只 |
| 冲高未涨停 | {chonggao_count}只 |
| 昨日涨停继续涨停 | {prev_zt_继续涨停}/{prev_zt_count} ({prev_zt_继续涨停/prev_zt_count*100:.1f}%) |
| 昨日涨停跌停 | {prev_zt_跌停}/{prev_zt_count} ({prev_zt_跌停/prev_zt_count*100:.1f}%) |

**市场情绪**: {'🔥 强' if 上涨/下跌 > 3 else '🔥 中' if 上涨/下跌 > 1.5 else '❄️ 弱'} | {'🚀 积极' if 封板率 > 70 else '⚠️ 谨慎' if 封板率 > 50 else '💀 悲观'}

"""

with open(OUTPUT_PATH, 'w', encoding='utf-8') as f:
    f.write(md)

print("=" * 50)
print(f"✅ 已生成: {OUTPUT_PATH}")
print(f"\n📊 核心数据:")
print(f"   涨跌比: {涨跌比}")
print(f"   涨停: {zt_count}只 | 跌停: {diting_count}只")
print(f"   封板率: {封板率:.1f}%")
print(f"   冲高未涨停: {chonggao_count}只")
print(f"   昨日涨停继续涨停: {prev_zt_继续涨停}/{prev_zt_count}")
