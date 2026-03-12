import akshare as ak
# 检查实时行情字段
df = ak.stock_zh_a_spot_em()
print("字段:", df.columns.tolist()[:15])
# 跌停筛选
dt = df[df['涨跌幅'] <= -9.9]
print(f"跌停数: {len(dt)}")
print(dt[['代码','名称','涨跌幅']].head())
