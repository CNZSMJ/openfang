import akshare as ak
# 试试不同的接口
try:
    df = ak.index_zh_a_hist(symbol="000001", period="daily", start_date="20260301", end_date="20260310")
    print("index_zh_a_hist:")
    print(df.columns.tolist())
    print(df.tail(2))
except Exception as e:
    print(f"error: {e}")
