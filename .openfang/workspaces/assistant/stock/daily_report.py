#!/usr/bin/env python3
"""
A股超短复盘系统 v2.0
每日收盘生成完整复盘报告
"""

import akshare as ak
import pandas as pd
import sys
import os
from datetime import datetime

def get_trade_date():
    """获取上一个交易日"""
    if len(sys.argv) > 1:
        return sys.argv[1]
    return datetime.now().strftime("%Y%m%d")

def get_zt_pool(date):
    """获取涨停板数据"""
    try:
        return ak.stock_zt_pool_em(date=date)
    except:
        return pd.DataFrame()

def get_dt_pool(date):
    """获取跌停板数据"""
    try:
        return ak.stock_zt_pool_dtgc_em(date=date)
    except:
        return pd.DataFrame()

def analyze_emotion(df_zt, df_dt, df_spot, total_amt):
    """情绪分析"""
    zt_count = len(df_zt)
    dt_count = len(df_dt)
    zhaban = df_zt['炸板次数'].sum() if len(df_zt) > 0 else 0
    fb_rate = zt_count / (zt_count + zhaban) * 100 if (zt_count + zhaban) > 0 else 0
    
    up = len(df_spot[df_spot['涨跌幅'] > 0])
    down = len(df_spot[df_spot['涨跌幅'] < 0])
    red_rate = up / (up + down) * 100 if (up + down) > 0 else 0
    
    return {
        'zt_count': zt_count,
        'dt_count': dt_count,
        'zhaban': zhaban,
        'fb_rate': fb_rate,
        'red_rate': red_rate,
        'up': up,
        'down': down,
        'total_amt': total_amt
    }

def get_mood_level(emotion):
    """判断情绪阶段"""
    zt = emotion['zt_count']
    fb = emotion['fb_rate']
    red = emotion['red_rate']
    dt = emotion['dt_count']
    
    if zt >= 60 and fb >= 60 and red >= 70 and dt <= 3:
        return "🔥 主升高潮", "6-8", "疯狂模式，找最强干"
    elif zt >= 40 and fb >= 50 and red >= 60:
        return "📈 震荡上行", "4-6", "轻指数重个股"
    elif zt >= 25 and fb >= 40:
        return "➡️ 震荡分化", "3-5", "小心应对"
    elif zt >= 15 and fb >= 30:
        return "⚠️ 情绪退潮", "2-4", "管住手"
    else:
        return "❄️ 冰点区域", "1-2", "别做了"

def get_board_leaders(df_zt, top_boards):
    """获取板块龙头"""
    leaders = []
    for board in top_boards:
        board_stocks = df_zt[df_zt['所属行业'] == board]
        if len(board_stocks) == 0:
            continue
        lb = board_stocks[board_stocks['连板数'] > 1]
        if len(lb) > 0:
            leader = lb.sort_values('连板数', ascending=False).iloc[0]
        else:
            leader = board_stocks.sort_values('成交额', ascending=False).iloc[0]
        leaders.append({
            'board': board,
            'code': leader['代码'],
            'name': leader['名称'],
            'lb': int(leader['连板数']),
            'amt': leader['成交额'] / 1e8,
            'turnover': leader['换手率']
        })
    return leaders

def get_space_board(df_zt):
    """空间板（最高板）"""
    if len(df_zt) == 0:
        return None
    df = df_zt.sort_values('连板数', ascending=False).iloc[0]
    return {
        'code': df['代码'],
        'name': df['名称'],
        'lb': int(df['连板数']),
        'board': df['所属行业'],
        'fd_rate': df['封板资金'] / df['流通市值'] * 100 if df['流通市值'] > 0 else 0
    }

def get_verified_boards(df_zt, space_code):
    """验证过的2-3连板（不是最高板）"""
    if len(df_zt) == 0:
        return []
    df = df_zt[(df_zt['连板数'] >= 2) & (df_zt['连板数'] <= 3)]
    if space_code:
        df = df[df['代码'] != space_code]
    df = df.copy()
    df['score'] = df['连板数'] * 2 + df['成交额'].apply(lambda x: min(x/10e8, 5))
    return df.sort_values('score', ascending=False).head(5)

def get_20cm(df_zt):
    """20cm弹性票"""
    if len(df_zt) == 0:
        return []
    return df_zt[df_zt['涨跌幅'] > 15].sort_values('成交额', ascending=False).head(5)

def get_followers(df_zt, space_board):
    """跟风股（同一板块，低于空间板）"""
    if not space_board or len(df_zt) == 0:
        return []
    board = space_board['board']
    return df_zt[(df_zt['所属行业'] == board) & (df_zt['代码'] != space_board['code'])].head(5)

def get_risk_stocks(df_spot):
    """风险标的：冲高回落 + 跌停"""
    cg = df_spot[(df_spot['涨跌幅'] > 5) & (df_spot['涨跌幅'] < 9.5)].sort_values('涨跌幅', ascending=False).head(10)
    dt = df_spot[df_spot['涨跌幅'] <= -9.5]
    return cg, dt

def generate_report(date):
    print(f"📊 正在生成 {date} 复盘报告...")
    
    # ===== 1. 指数数据 =====
    df_sh = ak.stock_zh_index_daily(symbol="sh000001")
    df_sz = ak.stock_zh_index_daily(symbol="sz399001")
    df_cy = ak.stock_zh_index_daily(symbol="sz399006")
    
    sh = {'close': df_sh.iloc[-1]['close'], 'pct': (df_sh.iloc[-1]['close']/df_sh.iloc[-1]['open']-1)*100}
    sz = {'close': df_sz.iloc[-1]['close'], 'pct': (df_sz.iloc[-1]['close']/df_sz.iloc[-1]['open']-1)*100}
    cy = {'close': df_cy.iloc[-1]['close'], 'pct': (df_cy.iloc[-1]['close']/df_cy.iloc[-1]['open']-1)*100}
    
    # ===== 2. 全市场数据 =====
    df_spot = ak.stock_zh_a_spot_em()
    total_amt = df_spot['成交额'].sum() / 1e8
    
    # ===== 3. 涨停板 =====
    df_zt = get_zt_pool(date)
    
    # ===== 4. 跌停板 =====
    df_dt = get_dt_pool(date)
    
    # ===== 5. 情绪分析 =====
    emotion = analyze_emotion(df_zt, df_dt, df_spot, total_amt)
    mood, cang_range, advice = get_mood_level(emotion)
    
    # 解析仓位
    cang_parts = cang_range.split('-')
    cang_min = cang_parts[0]
    cang_max = cang_parts[1]
    
    # ===== 6. 板块分析 =====
    board_cnt = df_zt['所属行业'].value_counts() if len(df_zt) > 0 else pd.Series()
    top_boards = board_cnt.head(5).index.tolist()
    
    # ===== 7. 龙头股 =====
    leaders = get_board_leaders(df_zt, top_boards[:3])
    space_board = get_space_board(df_zt)
    verified = get_verified_boards(df_zt, space_board['code'] if space_board else None)
    followers = get_followers(df_zt, space_board)
    cm_20 = get_20cm(df_zt)
    
    # ===== 8. 风险标的 =====
    cg, dt_risk = get_risk_stocks(df_spot)
    
    # ===== 9. 日期格式化 =====
    date_fmt = f"{date[:4]}-{date[4:6]}-{date[6:]}"
    
    # ===== 10. 明日策略 =====
    tomorrow_focus = top_boards[0] if top_boards else "无"
    tomorrow_buy = leaders[0]['name'] if leaders else "无"
    tomorrow_20cm = cm_20.iloc[0]['名称'] if len(cm_20) > 0 else "无"
    tomorrow_verified = verified.iloc[0]['名称'] if len(verified) > 0 else "无"
    focus_cnt = int(board_cnt.iloc[0] // 2) if len(board_cnt) > 0 else 0
    
    space_name = space_board['name'] if space_board else "无"
    
    # ===== 生成Markdown =====
    output = f"""# 📊 {date_fmt} 超短复盘

---

## 一、指数表现

| 指数 | 收盘 | 涨跌 | 判断 |
|------|------|------|------|
| 沪指 | {sh['close']:.2f} | {sh['pct']:+.2f}% | {'稳' if sh['pct'] > 0 else '弱'} |
| 深成指 | {sz['close']:.2f} | {sz['pct']:+.2f}% | {'跟' if sz['pct'] > 0 else '弱'} |
| 创业板 | {cy['close']:.2f} | {cy['pct']:+.2f}% | {'弹' if cy['pct'] > 0 else '弱'} |

---

## 二、情绪核心指标

| 指标 | 数值 | 信号 |
|------|------|------|
| 涨跌比 | {emotion['up']}:{emotion['down']}（{emotion['red_rate']:.0f}%红） | {'多头控盘' if emotion['red_rate'] > 60 else '分歧'} |
| 涨停/跌停 | {emotion['zt_count']}/{emotion['dt_count']} | {'强' if emotion['zt_count'] > 40 else '弱'} |
| 封板率 | {emotion['fb_rate']:.0f}% | {'有肉' if emotion['fb_rate'] > 50 else '难打'} |
| 炸板数 | {emotion['zhaban']} | {'多' if emotion['zhaban'] > 20 else '少'} |
| 成交额 | {emotion['total_amt']:.0f}亿 | {'活跃' if emotion['total_amt'] > 15000 else '缩量'} |

### 情绪定位：**{mood}**
> {advice}
### 建议仓位：**{cang_range}成**

---

## 三、主线板块（涨停数TOP5）

| 排名 | 板块 | 涨停数 | 占比 | 强度 |
|------|------|--------|------|------|
"""

    for i, board in enumerate(top_boards[:5]):
        cnt = board_cnt.get(board, 0)
        pct = cnt / emotion['zt_count'] * 100 if emotion['zt_count'] > 0 else 0
        strength = '🔥强' if cnt >= 8 else '一般'
        output += f"| {i+1} | {board} | {cnt} | {pct:.0f}% | {strength} |\n"

    output += f"""
---

## 四、空间板（最高板）

| 代码 | 名称 | 连板 | 板块 | 封单比 | 操作建议 |
|------|------|------|------|--------|----------|
"""

    if space_board:
        risk = '⚠️ 持有可走' if space_board['lb'] >= 4 else '可试错'
        output += f"| {space_board['code']} | **{space_board['name']}** | {space_board['lb']}连板 | {space_board['board']} | {space_board['fd_rate']:.1f}% | {risk} |\n"

    output += f"""
---

## 五、板块龙头（每个板块最强1只）

| 板块 | 代码 | 名称 | 连板 | 成交额 | 买法 |
|------|------|------|------|--------|------|
"""

    for l in leaders:
        buy = "低吸" if l['turnover'] > 20 else "高开打"
        output += f"| {l['board']} | {l['code']} | {l['name']} | {l['lb']}连板 | {l['amt']:.1f}亿 | {buy} |\n"

    output += f"""
---

## 六、验证过的2-3连板（已封住，非最高板）

> 逻辑：已经市场验证的强势股，分歧时低吸

| 代码 | 名称 | 板块 | 连板 | 成交额 | 换手 |
|------|------|------|------|--------|------|
"""

    for _, row in verified.iterrows():
        output += f"| {row['代码']} | {row['名称']} | {row['所属行业']} | {int(row['连板数'])}连板 | {row['成交额']/1e8:.1f}亿 | {row['换手率']:.1f}% |\n"

    output += f"""
---

## 七、跟风补涨（同一板块，低于空间板）

| 代码 | 名称 | 涨跌幅 | 成交额 | 备注 |
|------|------|--------|--------|------|
"""

    for _, row in followers.iterrows():
        output += f"| {row['代码']} | {row['名称']} | {row['涨跌幅']:.1f}% | {row['成交额']/1e8:.1f}亿 | 跟风 |\n"

    output += f"""
---

## 八、20cm弹性票

| 代码 | 名称 | 涨停时间 | 换手 | 板块 |
|------|------|----------|------|------|
"""

    for _, row in cm_20.iterrows():
        t = str(row['首次封板时间'])
        t_fmt = f"{t[:2]}:{t[2:4]}" if len(t) >= 4 else t
        output += f"| {row['代码']} | {row['名称']} | {t_fmt} | {row['换手率']:.1f}% | {row['所属行业']} |\n"

    output += f"""
---

## 九、亏钱效应（风险警示）

### 冲高回落（5%-9.5%，有抛压）
| 代码 | 名称 | 收盘涨幅 | 风险 |
|------|------|----------|------|
"""

    for _, row in cg.iterrows():
        output += f"| {row['代码']} | {row['名称']} | {row['涨跌幅']:.1f}% | 明天有抛压 |\n"

    output += f"""
### 跌停（别抄底）
| 代码 | 名称 | 收盘 |
|------|------|------|
"""

    for _, row in dt_risk.iterrows():
        output += f"| {row['代码']} | {row['名称']} | {row['涨跌幅']:.1f}% |\n"

    output += f"""
---

## 十、明日操作计划

**开盘三件事**：
1. 看{space_name}开盘情况（高开3%+企稳可打）
2. 看{tomorrow_focus}板块有{focus_cnt}只快速板
3. 看封板率能否回升到50%+

**买什么**：
- 龙头：{tomorrow_buy}
- 验证连板：{tomorrow_verified}
- 20cm：{tomorrow_20cm}

**仓位分配**：
- 龙头 {cang_max}成
- 验证板 {cang_min}成

**卖点纪律**：
- 盈利5%→卖1/3，7%→再卖1/3
- 亏损-3%→无条件砍
- 收盘不涨停→清

---

*报告生成：{date_fmt}*
"""

    # 保存
    report_dir = os.path.dirname(os.path.abspath(__file__))
    report_path = os.path.join(report_dir, f"report_{date}.md")
    with open(report_path, 'w') as f:
        f.write(output)
    
    latest = os.path.join(report_dir, "report_latest.md")
    with open(latest, 'w') as f:
        f.write(output)
    
    print(f"✅ 已生成: {report_path}")
    return output

if __name__ == "__main__":
    date = get_trade_date()
    generate_report(date)
