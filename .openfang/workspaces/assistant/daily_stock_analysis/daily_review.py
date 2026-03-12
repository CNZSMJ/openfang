#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
A股每日复盘工具
结合复盘策略：大盘概况 → 涨停归类 → 跌停分析 → 总结
"""

import akshare as ak
import pandas as pd
from datetime import datetime, timedelta
from collections import Counter

# ===== 配置 =====
def get_latest_trading_date():
    """获取最近交易日（跳过周末）"""
    today = datetime.now()
    if today.weekday() == 5:
        today -= timedelta(days=1)
    elif today.weekday() == 6:
        today -= timedelta(days=2)
    return today.strftime("%Y%m%d")

# ===== 数据获取 =====
def get_market_overview(date):
    """获取大盘指数（解决数据延迟问题）"""
    indices = {}
    
    # 获取三个指数数据
    df_sh = ak.stock_zh_index_daily(symbol="sh000001")
    df_sz = ak.stock_zh_index_daily(symbol="sz399001")
    df_cy = ak.stock_zh_index_daily(symbol="sz399006")
    
    # 找到共同最新交易日（取所有指数最新日期的最小值，解决数据延迟问题）
    latest_dates = [
        pd.to_datetime(df_sh['date']).max(),
        pd.to_datetime(df_sz['date']).max(),
        pd.to_datetime(df_cy['date']).max()
    ]
    common_latest = min(latest_dates)
    
    # 获取前一个交易日
    df_all = pd.concat([df_sh[['date']], df_sz[['date']], df_cy[['date']]])
    df_all['date'] = pd.to_datetime(df_all['date'])
    df_all = df_all.drop_duplicates().sort_values('date')
    dates_list = df_all['date'].tolist()
    
    try:
        prev_date = dates_list[dates_list.index(common_latest) - 1]
    except:
        prev_date = dates_list[-2]  # fallback
    
    # 计算各指数涨跌幅
    for name, df in [('沪指', df_sh), ('深成指', df_sz), ('创业板', df_cy)]:
        df['date'] = pd.to_datetime(df['date'])
        latest = df[df['date'] == common_latest].iloc[-1]
        prev = df[df['date'] == prev_date].iloc[-1]
        change_pct = (latest['close'] - prev['close']) / prev['close'] * 100
        indices[name] = {'close': latest['close'], 'pct': change_pct}
    
    return indices

def get_zt_pool(date):
    """获取涨停股池"""
    try:
        return ak.stock_zt_pool_em(date=date)
    except:
        return pd.DataFrame()

def get_market_stats():
    """获取市场涨跌统计"""
    try:
        df = ak.stock_zh_a_spot_em()
        up = len(df[df['涨跌幅'] > 0])
        down = len(df[df['涨跌幅'] < 0])
        flat = len(df[df['涨跌幅'] == 0])
        return {'up': up, 'down': down, 'flat': flat, 'total': len(df)}
    except:
        return {'up': 0, 'down': 0, 'flat': 0, 'total': 0}

# ===== 复盘策略核心 =====
def analyze_hot_sectors(zt_df):
    """热点板块归类"""
    if zt_df.empty:
        return {}
    
    sector_count = zt_df.groupby('所属行业').size().sort_values(ascending=False)
    
    hot_sectors = {}
    for sector in sector_count.head(5).index:
        stocks = zt_df[zt_df['所属行业'] == sector][['代码', '名称', '涨跌幅', '连板数', '涨停统计']].head(8)
        hot_sectors[sector] = stocks.to_dict('records')
    
    return hot_sectors

def analyze_dt_stocks(zt_df):
    """跌停股分析"""
    if zt_df.empty:
        return []
    dt = zt_df[zt_df['涨跌幅'] <= -9.9]
    if dt.empty:
        return []
    return dt[['代码', '名称', '涨跌幅', '所属行业']].to_dict('records')

def calculate_sealing_rate(zt_df):
    """计算封板率"""
    if zt_df.empty:
        return 0.0
    sealed = len(zt_df[zt_df['涨跌幅'] >= 9.9])
    total = len(zt_df)
    return sealed / total * 100 if total > 0 else 0.0

# ===== 报告生成 =====
def generate_report(date, indices, zt_df, market_stats):
    zt_count = len(zt_df)
    dt_df = zt_df[zt_df['涨跌幅'] <= -9.9]
    dt_count = len(dt_df)
    sealing_rate = calculate_sealing_rate(zt_df)
    
    hot_sectors = analyze_hot_sectors(zt_df)
    dt_stocks = analyze_dt_stocks(zt_df)
    
    avg_change = sum(v['pct'] for v in indices.values()) / len(indices)
    if avg_change > 1:
        mood = "大幅上涨"
    elif avg_change > 0:
        mood = "小幅上涨"
    elif avg_change > -1:
        mood = "小幅下跌"
    else:
        mood = "明显下跌"
    
    report = f"""# {date[:4]}年{date[4:6]}月{date[6:]}日 A股复盘

> 数据来源：akshare | 生成时间：{datetime.now().strftime('%H:%M')}

---

## 一、大盘概况

| 指数 | 收盘价 | 涨跌幅 |
|------|--------|--------|
| 沪指 | {indices['沪指']['close']:.2f} | {indices['沪指']['pct']:+.2f}% |
| 深成指 | {indices['深成指']['close']:.2f} | {indices['深成指']['pct']:+.2f}% |
| 创业板 | {indices['创业板']['close']:.2f} | {indices['创业板']['pct']:+.2f}% |

**涨停：{zt_count}只 | 跌停：{dt_count}只 | 封板率：{sealing_rate:.0f}%**

**涨跌统计**：涨 {market_stats['up']} | 跌 {market_stats['down']} | 平 {market_stats['flat']}

**行情描述**：今日市场{mood}。

"""
    
    report += "## 二、涨停板块与个股\n\n"
    for i, (sector, stocks) in enumerate(hot_sectors.items(), 1):
        sector_zt_count = len(zt_df[zt_df['所属行业'] == sector])
        report += f"### {i}. {sector} ({sector_zt_count}只)\n\n"
        report += "| 代码 | 名称 | 涨幅 | 连板 | 封板 |\n"
        report += "|------|------|------|------|------|\n"
        for s in stocks:
            lb = s.get('连板数', '-')
            stat = s.get('涨停统计', '-')
            report += f"| {s['代码']} | {s['名称']} | {s['涨跌幅']:.2f}% | {lb} | {stat} |\n"
        report += "\n"
    
    report += "## 三、跌停股分析\n\n"
    if dt_stocks:
        report += "| 代码 | 名称 | 跌幅 | 所属行业 |\n"
        report += "|------|------|------|----------|\n"
        for s in dt_stocks:
            report += f"| {s['代码']} | {s['名称']} | {s['涨跌幅']:.2f}% | {s['所属行业']} |\n"
    else:
        report += "无跌停股\n"
    
    report += """
## 四、总结

### 赚钱效应
"""
    if hot_sectors:
        main_sector = list(hot_sectors.keys())[0]
        report += f"- 热点板块：{main_sector}\n"
        report += f"- 涨停数：{zt_count}只，封板率{sealing_rate:.0f}%\n"
    else:
        report += "- 无明显热点\n"
    
    report += """
### 亏钱效应
"""
    if dt_stocks:
        sectors = Counter([s['所属行业'] for s in dt_stocks])
        worst = sectors.most_common(1)[0]
        report += f"- 领跌板块：{worst[0]}（{worst[1]}只跌停）\n"
    else:
        report += "- 无跌停股，市场情绪稳定\n"
    
    report += """
### 策略建议
- 关注主线板块龙头
- 回避弱势板块
- 控制仓位
"""
    
    return report

# ===== 主程序 =====
def main():
    date = get_latest_trading_date()
    print(f"正在复盘 {date}...")
    
    print("获取大盘指数...")
    indices = get_market_overview(date)
    
    print("获取涨停股池...")
    zt_df = get_zt_pool(date)
    
    print("获取市场统计...")
    market_stats = get_market_stats()
    
    report = generate_report(date, indices, zt_df, market_stats)
    
    output_file = f"output/{date}.md"
    with open(output_file, 'w', encoding='utf-8') as f:
        f.write(report)
    
    print(f"复盘完成: {output_file}")
    print(f"涨停: {len(zt_df)}只")

if __name__ == "__main__":
    main()
