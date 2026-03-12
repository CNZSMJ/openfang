import akshare as ak

# 看涨停板里通信设备股票的基本面
df_zt = ak.stock_zt_pool_em(date='20260310')

# 通信设备股票
print("=== 通信设备股票 ===")
for _, row in df_zt[df_zt['所属行业']=='通信设备'].iterrows():
    code = row['代码']
    name = row['名称']
    print(f"{code} {name}")

# 看看这些股票在东财是什么行业
import pandas as pd
df_spot = ak.stock_zh_a_spot_em()

for code in ['603803', '603421', '601869', '002281', '600345']:
    stock = df_spot[df_spot['代码'] == code]
    if len(stock) > 0:
        print(f"{code}: {stock.iloc[0]['名称']}, 所属行业: {stock.iloc[0]['所属行业']}, 所属板块: {stock.iloc[0]['所属板块']}")
