import akshare as ak
import pandas as pd

# 昨日涨停 → 今天是涨还是跌
df_prev = ak.stock_zt_pool_previous_em(date="20260309")
print("昨日涨停今日表现:")
print(df_prev[['名称', '代码', '涨跌幅']].head(10))

# 找出今日跌停的（从昨日涨停里）
跌停 = df_prev[df_prev['涨跌幅'] < -9]
print(f"\n昨日涨停今日跌停: {len(跌停)}只")
print(跌停[['名称', '代码', '涨跌幅']])

# 涨跌停统计
print(f"\n=== 涨跌停统计 ===")
print(f"昨日涨停: {len(df_prev)}只")
print(f"今日涨停: 42只")
print(f"昨日涨停今日继续涨停: {len(df_prev[df_prev['涨跌幅'] > 9])}只")
print(f"昨日涨停今日跌停: {len(跌停)}只")

# 尝试找每日涨跌统计
# 东方财富行情中心
try:
    df = ak.stock_zh_a_sgt_em()
    print("\n个股数据概览列:", df.columns.tolist()[:15])
except Exception as e:
    print(f"error: {e}")
