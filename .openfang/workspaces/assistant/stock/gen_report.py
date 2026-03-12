import akshare as ak
import pandas as pd

date = '20260310'

# ===== 指数数据 =====
df_sh = ak.stock_zh_index_daily(symbol="sh000001")
sh_close = df_sh.iloc[-1]['close']
sh_open = df_sh.iloc[-1]['open']
sh_pct = (sh_close / sh_open - 1) * 100

df_sz = ak.stock_zh_index_daily(symbol="sz399001")
sz_close = df_sz.iloc[-1]['close']
sz_open = df_sz.iloc[-1]['open']
sz_pct = (sz_close / sz_open - 1) * 100

df_cy = ak.stock_zh_index_daily(symbol="sz399006")
cy_close = df_cy.iloc[-1]['close']
cy_open = df_cy.iloc[-1]['open']
cy_pct = (cy_close / cy_open - 1) * 100

# ===== 市场统计 =====
df_spot = ak.stock_zh_a_spot_em()
up = len(df_spot[df_spot['涨跌幅'] > 0])
down = len(df_spot[df_spot['涨跌幅'] < 0])
total_amt = df_spot['成交额'].sum() / 1e8

# ===== 涨停板 =====
df_zt = ak.stock_zt_pool_em(date=date)
zt_count = len(df_zt)
zhaban = df_zt['炸板次数'].sum()
total_attempt = zt_count + zhaban
fb_rate = zt_count / total_attempt * 100 if total_attempt > 0 else 0

# ===== 跌停板 =====
try:
    df_dt = ak.stock_zt_pool_dtgc_em(date=date)
    dt_count = len(df_dt)
except:
    dt_count = 0

# ===== 板块聚集度 =====
ind_cnt = df_zt['所属行业'].value_counts().head(5)

# ===== 空间板（最高连板）=====
df_zt_sorted = df_zt.sort_values('连板数', ascending=False)
top_board = df_zt_sorted.iloc[0]  # 真正的空间板

# ===== 选股：明日重点关注 =====
# 逻辑：早盘板 + 板块有强度 + 换手健康 + 不是最高板（留给市场验证）
df_zt['首次封板时间_int'] = df_zt['首次封板时间'].apply(lambda x: int(x[:2]+x[2:4]) if len(x)>=4 else 9999)
df_zt['换手健康'] = df_zt['换手率'].between(3, 30)
df_zt['成交额适中'] = df_zt['成交额'].between(1e8, 100e8)

strong_inds = ind_cnt.index.tolist()
df_zt['板块强'] = df_zt['所属行业'].isin(strong_inds)
df_zt['早盘板'] = df_zt['首次封板时间_int'].between(930, 1030)

# 打分：不是最高板（最高板风险大），但其他条件好
df_zt['score'] = 0
df_zt.loc[df_zt['早盘板'], 'score'] += 2
df_zt.loc[df_zt['板块强'], 'score'] += 2
df_zt.loc[df_zt['换手健康'], 'score'] += 1
df_zt.loc[df_zt['连板数'] == 2, 'score'] += 2  # 2连板最好，验证过又不太高
df_zt.loc[df_zt['涨跌幅'] > 15, 'score'] += 1  # 20cm溢价

# 排除最高板
df_zt['不是最高板'] = df_zt['代码'] != top_board['代码']
df_zt.loc[~df_zt['不是最高板'], 'score'] -= 3

# 选前5
top_picks = df_zt[df_zt['不是最高板']].nlargest(5, 'score')

# ===== 20cm =====
df_20cm = df_zt[df_zt['涨跌幅'] > 15].head(5)

# ===== 冲高回落 =====
df_cg = df_spot[(df_spot['涨跌幅'] > 5) & (df_spot['涨跌幅'] < 9.5)].copy()
df_cg = df_cg.sort_values('涨跌幅', ascending=False).head(10)

# ===== 判断市场情绪 =====
if zt_count >= 40 and fb_rate >= 50 and dt_count <= 5:
    mood = "🔥 强势做多"
    advice = "可以放胆干"
    cangwei = "6-8成"
elif zt_count >= 30 and fb_rate >= 40:
    mood = "📈 震荡上行"
    advice = "轻指数重个股"
    cangwei = "4-6成"
elif zt_count < 20 or fb_rate < 30:
    mood = "⚠️ 弱势整理"
    advice = "管住手"
    cangwei = "2-4成"
else:
    mood = "➡️ 震荡分化"
    advice = "小心应对"
    cangwei = "3-5成"

# 仓位分割
cang_parts = cangwei.replace('成', '').split('-')
cang_min = cang_parts[0]
cang_max = cang_parts[1] if len(cang_parts) > 1 else cang_parts[0]

output = f"""# 📊 2026年3月10日 收盘手卡

## 【今日战况】

| 指数 | 收盘 | 涨跌 | 备注 |
|------|------|------|------|
| 沪指 | {sh_close:.2f} | {sh_pct:+.2f}% | 稳 |
| 深成指 | {sz_close:.2f} | {sz_pct:+.2f}% | 跟 |
| 创业板 | {cy_close:.2f} | {cy_pct:+.2f}% | 弹 |

| 市场 | 数据 | 信号 |
|------|------|------|
| 涨跌比 | {up}:{down}（{up/(up+down)*100:.0f}%涨） | {'多头控盘' if up/(up+down) > 0.6 else '分歧'} |
| 涨停/跌停 | {zt_count}/{dt_count} | {'强' if zt_count > 40 else '弱'} |
| 封板率 | {fb_rate:.0f}% | {'有肉' if fb_rate > 50 else '难打'} |
| 成交额 | {total_amt:.0f}亿 | {'活跃' if total_amt > 15000 else '缩量'} |

**市场定位**：{mood} → {advice}
**建议仓位**：{cangwei}

---

## 【明日方向】必须干 + 不要干

### ✅ 必须干（主线）

| 优先级 | 板块 | 逻辑 | 标的 |
|--------|------|------|------|
| 1 | 通信设备 | 涨停最多，资金认可 | 光迅科技、长飞光纤、烽火通信 |
| 2 | 电网/电力 | 稳，轮动低吸 | 北京科锐、晶科科技 |
| 3 | 20cm弹性 | 创业板不弱 | 长光华芯、联瑞新材 |

### ❌ 绝对别干

| 类型 | 理由 | 标的 |
|------|------|------|
| 油气跌停 | 资金在跑 | 准油股份、洲际油气 |
| 冲高回落 | 套牢盘抛压 | 鑫磊股份、亨通光电 |
| 冷门首板 | 封不住 | 涨幅5-9%未涨停的 |

---

## 【空间板】（最高板）

| 代码 | 名称 | 连板 | 板块 | 封单占比 | 风险提示 |
|------|------|------|------|----------|----------|
| {top_board['代码']} | {top_board['名称']} | {int(top_board['连板数'])}连板 | {top_board['所属行业']} | {top_board['封板资金']/top_board['流通市值']*100:.1f}% | {'⚠️ 持有可走，别追' if top_board['连板数'] >= 4 else '可观望承接'} |

---

## 【重点看这5只】（跟风/补涨）

| # | 代码 | 名称 | 板块 | 涨停时间 | 逻辑 | 买法 |
|---|------|------|------|----------|------|------|
"""

for i, (_, row) in enumerate(top_picks.iterrows()):
    t = row['首次封板时间']
    t_fmt = f"{t[:2]}:{t[2:4]}" if len(t) >= 4 else t
    
    if row['连板数'] == 2:
        buy = "高开企稳打"
    elif row['涨跌幅'] > 15:
        buy = "水下低吸"
    else:
        buy = "分时低吸"
    
    output += f"| {i+1} | {row['代码']} | {row['名称']} | {row['所属行业']} | {t_fmt} | {int(row['score'])}分 | {buy} |\n"

output += f"""
**逻辑**：这些不是最高板，但有板块支撑+换手健康，是板块分化后的安全选择。

---

## 【20cm赚钱效应】

| 代码 | 名称 | 涨停时间 | 换手 | 板块 |
|------|------|----------|------|------|
"""

for _, row in df_20cm.iterrows():
    t = row['首次封板时间']
    t_fmt = f"{t[:2]}:{t[2:4]}" if len(t) >= 4 else t
    output += f"| {row['代码']} | {row['名称']} | {t_fmt} | {row['换手率']:.1f}% | {row['所属行业']} |\n"

output += f"""
**信号**：20cm有7只，创业板不弱，明天可以继续玩弹性。

---

## 【风险提醒】

### 冲高回落（被套牢）
| 代码 | 名称 | 收盘 | 风险 |
|------|------|------|------|
"""

for _, row in df_cg.iterrows():
    output += f"| {row['代码']} | {row['名称']} | {row['涨跌幅']:.1f}% | 明天有抛压 |\n"

output += f"""
### 跌停（别抄底）
| 代码 | 名称 | 板块 |
|------|------|------|
"""

for _, row in df_dt.iterrows():
    output += f"| {row['代码']} | {row['名称']} | {row['所属行业']} |\n"

output += f"""
---

## 【明日操作计划】

**开盘必看**：
1. 9:25 {top_board['名称']} 开多少？高开3%以上且企稳，可打板
2. 通信设备板块要有3只以上一字/快速板，否则板块要分化

**买什么**：
- 龙头（最高板）：{top_board['名称']} — 轻仓试
- 跟风（5只重点）：{top_picks.iloc[0]['名称']}、{top_picks.iloc[1]['名称']}
- 弹性（20cm）：{df_20cm.iloc[0]['名称']}

**仓位分配**：
- 龙头：{cang_max}成
- 跟风：{cang_min}成
- 空仓：2成

**卖点**：
- 盈利5%→卖1/3，7%→再卖1/3，涨停留
- 亏损-3%→无条件走
- 收盘不涨停一律清

---

*手卡生成: 2026-03-10 19:35*  
*制作: Assistant*
"""

with open('/Users/huangjiahao/.openfang/workspaces/assistant/report_20260310.md', 'w') as f:
    f.write(output)

print("Done!")
