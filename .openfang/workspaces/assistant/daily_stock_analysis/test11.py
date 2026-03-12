import akshare as ak
import pandas as pd

# 用行业板块数据统计涨跌比
df = ak.stock_board_industry_name_em()
print("行业板块数据:")
print(df[['板块名称', '上涨家数', '下跌家数']].head(10))

# 汇总
上涨 = df['上涨家数'].sum()
下跌 = df['下跌家数'].sum()
total = 上涨 + 下跌

print(f"\n=== 涨跌比统计 ===")
print(f"上涨: {上涨}只")
print(f"下跌: {下跌}只")
print(f"总计: {total}只")
if 下跌 > 0:
    print(f"涨跌比: {上涨}:{下跌} = {上涨/下跌:.2f}:1")
else:
    print("涨跌比: N/A")
