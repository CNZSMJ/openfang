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

# ===== 板块聚集度（从涨停板统计）=====
ind_cnt = df_zt['所属行业'].value_counts()
top_inds = ind_cnt.head(3)  # 前3个板块

# ===== 空间板 =====
df_zt_sorted = df_zt.sort_values('连板数', ascending=False)
top_board = df_zt_sorted.iloc[0]

# ===== 板块内选龙头（每个板块选最强的1只）=====
def get_board_leader(board_name):
    """从指定板块选出最强票：连板数>1优先，否则按成交额"""
    board_stocks = df_zt[df_zt['所属行业'] == board_name].copy()
    if len(board_stocks) == 0:
        return None
    
    # 优先选连板的
    board_stocks_lb = board_stocks[board_stocks['连板数'] > 1]
    if len(board_stocks_lb) > 0:
        return board_stocks_lb.sort_values('连板数', ascending=False).iloc[0]
    else:
        # 没连板就选成交额最大的
        return board_stocks.sort_values('成交额', ascending=False).iloc[0]

# ===== 2-3连板（验证过能封住，不是最高板）=====
df_2to3 = df_zt[(df_zt['连板数'] >= 2) & (df_zt['连板数'] <= 3) & (df_zt['代码'] != top_board['代码'])].copy()
df_2to3['score'] = df_2to3['连板数'] * 2 + df_2to3['成交额'].apply(lambda x: min(x/10e8, 5))  # 连板+成交额
df_2to3 = df_2to3.sort_values('score', ascending=False).head(5)

# ===== 20cm =====
df_20cm = df_zt[df_zt['涨跌幅'] > 15].sort_values('成交额', ascending=False).head(5)

# ===== 冲高回落 =====
df_cg = df_spot[(df_spot['涨跌幅'] > 5) & (df_spot['涨跌幅'] < 9.5)].copy()
df_cg = df_cg.sort_values('涨跌幅', ascending=False).head(10)

# ===== 情绪判断 =====
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

cang_parts = cangwei.replace('成', '').split('-')
cang_min = cang_parts[0]
cang_max = cang_parts[1] if len(cang_parts) > 1 else cang_parts[0]

# ===== 预计算龙头和标的 =====
leader1 = get_board_leader(top_inds.index[0])
leader2 = get_board_leader(top_inds.index[1]) if len(top_inds) > 1 else None
leader3 = get_board_leader(top_inds.index[2]) if len(top_inds) > 2 else None

leader1_name = leader1['名称'] if leader1 is not None else '无'
leader1_code = leader1['代码'] if leader1 is not None else ''
leader1_lb = int(leader1['连板数']) if leader1 is not None else 0

leader2_name = leader2['名称'] if leader2 is not None else '无'
leader3_name = leader3['名称'] if leader3 is not None else '无'

pick_2to3_name = df_2to3.iloc[0]['名称'] if len(df_2to3) > 0 else '无'
pick_20cm_name = df_20cm.iloc[0]['名称'] if len(df_20cm) > 0 else '无'

# ===== 生成报告 =====
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

## 【主线板块】（涨停数排名前3）

| 排名 | 板块 | 涨停数 | 占比 | 板块强度 |
|------|------|--------|------|----------|
| 1 | {top_inds.index[0]} | {top_inds.iloc[0]} | {top_inds.iloc[0]/zt_count*100:.0f}% | {'强' if top_inds.iloc[0] >= 8 else '一般'} |
| 2 | {top_inds.index[1]} | {top_inds.iloc[1]} | {top_inds.iloc[1]/zt_count*100:.0f}% | {'强' if top_inds.iloc[1] >= 6 else '一般'} |
| 3 | {top_inds.index[2]} | {top_inds.iloc[2]} | {top_inds.iloc[2]/zt_count*100:.0f}% | {'强' if top_inds.iloc[2] >= 5 else '一般'} |

---

## 【板块龙头】（每个板块最强1只）

"""

# 每个板块选龙头
for i, board_name in enumerate(top_inds.index):
    leader = get_board_leader(board_name)
    if leader is not None:
        buy_tip = "水下低吸" if leader['换手率'] > 20 else "高开企稳打"
        output += f"| {i+1} | {board_name} | {leader['代码']} | {leader['名称']} | {int(leader['连板数'])}连板 | {leader['成交额']/1e8:.1f}亿 | {buy_tip} |\n"

output += f"""
---

## 【验证过的2-3连板】（已封住，不是最高板）

| 代码 | 名称 | 板块 | 连板 | 成交额 | 换手 | 买法 |
|------|------|------|------|--------|------|------|
"""

for _, row in df_2to3.iterrows():
    buy_tip = "高开企稳打" if row['连板数'] == 2 else "轻仓试"
    output += f"| {row['代码']} | {row['名称']} | {row['所属行业']} | {int(row['连板数'])}连板 | {row['成交额']/1e8:.1f}亿 | {row['换手率']:.1f}% | {buy_tip} |\n"

output += f"""
**逻辑**：2-3连板是市场验证过的强势股，不是最高板（风险可控），分歧时低吸。

---

## 【空间板】（最高板）

| 代码 | 名称 | 连板 | 板块 | 封单占比 | 风险提示 |
|------|------|------|------|----------|----------|
| {top_board['代码']} | {top_board['名称']} | {int(top_board['连板数'])}连板 | {top_board['所属行业']} | {top_board['封板资金']/top_board['流通市值']*100:.1f}% | {'⚠️ 持有可走，别追' if top_board['连板数'] >= 4 else '可观望承接'} |

---

## 【20cm弹性】（从涨停板里选）

| 代码 | 名称 | 涨停时间 | 换手 | 板块 |
|------|------|----------|------|------|
"""

for _, row in df_20cm.iterrows():
    t = row['首次封板时间']
    t_fmt = f"{t[:2]}:{t[2:4]}" if len(t) >= 4 else t
    output += f"| {row['代码']} | {row['名称']} | {t_fmt} | {row['换手率']:.1f}% | {row['所属行业']} |\n"

output += f"""
**信号**：20cm有{len(df_20cm)}只，创业板不弱。

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
2. {top_inds.index[0]} 板块要有{top_inds.iloc[0]//2}只以上快速板，否则板块要分化

**买什么**（全部从涨停板里选，不体外循环）：
- 板块龙头：{leader1_name}
- 验证连板：{pick_2to3_name}
- 弹性20cm：{pick_20cm_name}

**仓位分配**：
- 龙头：{cang_max}成
- 验证连板：{cang_min}成
- 空仓：2成

**卖点**：
- 盈利5%→卖1/3，7%→再卖1/3，涨停留
- 亏损-3%→无条件走
- 收盘不涨停一律清

---

*手卡生成: 2026-03-10*
"""

with open('/Users/huangjiahao/.openfang/workspaces/assistant/report_20260310_v2.md', 'w') as f:
    f.write(output)

print("Done!")
