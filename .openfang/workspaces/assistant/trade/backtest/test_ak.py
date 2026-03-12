import akshare as ak

print("akshare version:", ak.__version__)

# 测试主要接口
print("\n测试 stock_zh_a_hist:")
try:
    df = ak.stock_zh_a_hist(symbol="600519", period="daily", start_date="2023-12-01", end_date="2023-12-31", adjust="qfq")
    print(f"  结果: {len(df)} 行")
    if len(df) > 0:
        print(df.head(2))
except Exception as e:
    print(f"  错误: {e}")

print("\n测试 stock_zh_a_hist_min_em:")
try:
    df = ak.stock_zh_a_hist_min_em(symbol="600519", start_date="20231201", end_date="20231231")
    print(f"  结果: {len(df)} 行")
except Exception as e:
    print(f"  错误: {e}")
