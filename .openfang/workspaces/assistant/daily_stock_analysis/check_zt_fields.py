import akshare as ak
df = ak.stock_zt_pool_em(date="20260309")
print("字段:", df.columns.tolist())
print("\n前3行:")
print(df.head(3).to_string())
