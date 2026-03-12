import akshare as ak
import pandas as pd
import sys
import os

# 添加当前目录到path，确保能导入同级模块
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from eastmoney_money_flow import get_board_money_flow, format_money_flow_report

# ===== 日期配置 =====
# 支持命令行传入日期，如: python gen_report_v2.py 20260310
import argparse
parser = argparse.ArgumentParser()
parser.add_argument('date', nargs='?', default='20260310', help='日期，格式YYYYMMDD')
args = parser.parse_args()
date = args.date

print(f"=== 生成 {date} 手卡 ===")

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
    df_dt = pd.DataFrame()

# ===== [新增] 板块资金流向 =====
print("获取板块资金流向...")
money_flow_data = get_board_money_flow(20)  # 获取前20个板块
# 转换为dict方便查询
money_flow_dict = {item['板块名']: item for item in money_flow_data if item['板块名']}

def fmt_money(v):
    """格式化金额"""
    if v is None or v == 0:
        return "0"
    v = float(v)
    if abs(v) >= 1e8:
        return f"{v/1e8:.2f}亿"
    elif abs(v) >= 1e4:
        return f"{v/1e4:.2f}万"
    return f"{v:.0f}"

# ===== 板块聚集度（涨停板）+ 资金流向双重验证 =====
ind_cnt = df_zt['所属行业'].value_counts()

# [新增] 融合资金流向，计算综合得分
def calc_board_score(board_name, zt_count):
    """计算板块综合得分：涨停数(30%) + 资金(70%)"""
    score = 0
    
    # 涨停基础分：每1个涨停 = 10分
    zt_score = zt_count * 10
    
    # 资金加分：有资金流入才加分
    money_score = 0
    if board_name in money_flow_dict:
        mf = money_flow_dict[board_name]
        inflow = mf.get('主力净流入', 0) or 0
        if inflow > 0:
            # 资金每1亿 = 20分，上限100分
            money_score = min(inflow / 1e8 * 20, 100)
        elif inflow < 0:
            # 资金流出扣分
            money_score = max(inflow / 1e8 * 20, -50)
    
    # 综合得分
    score = zt_score * 0.3 + money_score * 0.7
    return score

# 计算每个涨停板块的综合得分
board_scores = []
for board_name, zt_num in ind_cnt.items():
    score = calc_board_score(board_name, zt_num)
    money_info = money_flow_dict.get(board_name, {})
    board_scores.append({
        '板块': board_name,
        '涨停数': zt_num,
        '资金流入': money_info.get('主力净流入', 0) or 0,
        '涨跌幅': money_info.get('涨跌幅', 0) or 0,
        '综合得分': score,
        '净流入最大股': money_info.get('主力净流入最大股', '')
    })

# 按综合得分排序
board_scores = sorted(board_scores, key=lambda x: x['综合得分'], reverse=True)
top_inds = pd.Series({b['板块']: b['涨停数'] for b in board_scores[:5]})

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

# ===== [新增] 资金暗线池（资金流入强但未涨停的潜力板块）=====
# 筛选：有资金流入、涨幅不大、还没成为主线
dark_pools = []
for board_name, mf in money_flow_dict.items():
    inflow = mf.get('主力净流入', 0) or 0
    change = mf.get('涨跌幅', 0) or 0
    # 条件：主力净流入 > 10亿，涨幅 < 5%，不是涨停板主线
    if inflow > 10e8 and change < 5 and board_name not in ind_cnt.index[:5].tolist():
        dark_pools.append({
            '板块': board_name,
            '主力净流入': inflow,
            '涨跌幅': change,
            '净流入最大股': mf.get('主力净流入最大股', '')
        })

dark_pools = sorted(dark_pools, key=lambda x: x['主力净流入'], reverse=True)[:5]

# ===== 情绪判断（加入资金因素）=====
# 统计资金流入板块数
money_inflow_count = len([b for b in board_scores if b['资金流入'] > 0])

if zt_count >= 40 and fb_rate >= 50 and dt_count <= 5 and money_inflow_count >= 8:
    mood = "🔥 强势做多"
    advice = "资金+情绪共振，可以放胆干"
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
leader1 = get_board_leader(top_inds.index[0]) if len(top_inds) > 0 else None
leader2 = get_board_leader(top_inds.index[1]) if len(top_inds) > 1 else None
leader3 = get_board_leader(top_inds.index[2]) if len(top_inds) > 2 else None

leader1_name = leader1['名称'] if leader1 is not None else '无'
leader1_code = leader1['代码'] if leader1 is not None else ''
leader1_lb = int(leader1['连板数']) if leader1 is not None else 0

leader2_name = leader2['名称'] if leader2 is not None else '无'
leader3_name = leader3['名称'] if leader3 is not None else '无'

pick_2to3_name = df_2to3.iloc[0]['名称'] if len(df_2to3) > 0 else '无'
pick_20cm_name = df_20cm.iloc[0]['名称'] if len(df_20cm) > 0 else '无'

# ===== 格式化日期用于显示 =====
display_date = f"{date[:4]}-{date[4:6]}-{date[6:]}"

# ===== 生成报告 =====
output = f"""# 📊 {display_date} 收盘手卡

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
| 资金流入板块 | {money_inflow_count}个 | {'强' if money_inflow_count >= 10 else '一般'} |

**市场定位**：{mood} → {advice}
**建议仓位**：{cangwei}

---

## 【板块资金流向】（主力资金）

| 排名 | 板块 | 主力净流入 | 涨跌幅 | 综合判断 |
|------|------|-----------|--------|----------|
"""

# 展示资金+涨停双重验证后的主线
for i, b in enumerate(board_scores[:5]):
    # 综合判断标签
    zt_tag = "🔥" if b['涨停数'] >= 5 else ("📈" if b['涨停数'] >= 3 else "")
    money_tag = "💰" if b['资金流入'] > 10e8 else ("➡️" if b['资金流入'] > 0 else "💔")
    tags = f"{zt_tag}{money_tag}".strip()
    
    output += f"| {i+1} | {b['板块']} | {fmt_money(b['资金流入'])} | {b['涨跌幅']:+.2f}% | {tags} {b['净流入最大股']} |\n"

output += f"""
> 解读：🔥=涨停多，💰=资金猛，💔=资金流出

---

## 【主线板块】（涨停+资金双重验证）

| 排名 | 板块 | 涨停数 | 资金流入 | 综合得分 | 龙头 |
|------|------|--------|----------|----------|------|
"""

for i, b in enumerate(board_scores[:5]):
    output += f"| {i+1} | {b['板块']} | {b['涨停数']} | {fmt_money(b['资金流入'])} | {b['综合得分']:.0f} | {b['净流入最大股']} |\n"

output += f"""
---

## 【板块龙头】（每个板块最强1只）

"""

# 每个板块选龙头
for i, board_name in enumerate(top_inds.index[:3]):
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

## 【资金暗线池】（潜力板块）

> 资金大幅流入但还没涨停，可能成为明日补涨方向

| 板块 | 主力净流入 | 涨幅 | 净流入最大股 |
|------|-----------|------|-------------|
"""

if dark_pools:
    for dp in dark_pools:
        output += f"| {dp['板块']} | {fmt_money(dp['主力净流入'])} | {dp['涨跌幅']:+.2f}% | {dp['净流入最大股']} |\n"
else:
    output += "| - | - | - | - |\n"

output += f"""
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
    output += f"| {row['代码']} | {row['名称']} | {row.get('所属行业', '-')} |\n"

output += f"""
---

## 【明日操作计划】

**开盘必看**：
1. 9:25 {top_board['名称']} 开多少？高开3%以上且企稳，可打板
2. {top_inds.index[0]} 板块要有{top_inds.iloc[0]//2}只以上快速板，否则板块要分化
3. 资金暗线池板块是否有异动拉升

**买什么**（涨停板+资金暗线双轨选股）：
- 板块龙头：{leader1_name}
- 验证连板：{pick_2to3_name}
- 弹性20cm：{pick_20cm_name}
"""

# 加入暗线池推荐
if dark_pools:
    output += f"- 资金暗线：{dark_pools[0]['板块']}（{dark_pools[0]['净流入最大股']}）\n"

output += f"""
**仓位分配**：
- 龙头：{cang_max}成
- 验证连板：{cang_min}成
- 暗线/空仓：2成

**卖点**：
- 盈利5%→卖1/3，7%→再卖1/3，涨停留
- 亏损-3%→无条件走
- 收盘不涨停一律清

---

*手卡生成: {display_date}*
"""

# 保存报告
output_path = f'/Users/huangjiahao/.openfang/workspaces/assistant/reports/report_{date}.md'
with open(output_path, 'w') as f:
    f.write(output)

# 同时生成一份latest
with open('/Users/huangjiahao/.openfang/workspaces/assistant/reports/report_latest.md', 'w') as f:
    f.write(output)

print(f"✅ Done! 报告已生成: {output_path}")
