import akshare as ak
df = ak.stock_zt_pool_em(date="20260309")
print("涨停池字段:", df.columns.tolist())
print(df.head(2))
