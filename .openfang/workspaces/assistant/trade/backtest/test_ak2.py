import akshare as ak

# 尝试不同方式
print("测试1: 不指定日期范围")
try:
    df = ak.stock_zh_a_hist(symbol="600519", adjust="qfq")
    print(f"  结果: {len(df)} 行")
    if len(df) > 0:
        print(df.tail(3))
except Exception as e:
    print(f"  错误: {e}")

print("\n测试2: 使用stock_zh_a_daily")
try:
    df = ak.stock_zh_a_daily(symbol="600519", adjust="qfq")
    print(f"  结果: {len(df)} 行")
except Exception as e:
    print(f"  错误: {e}")

print("\n测试3: 获取所有可以用的接口")
for attr in dir(ak):
    if '600519' in attr.lower() or 'hist' in attr.lower() or 'daily' in attr.lower():
        if not attr.startswith('_'):
            print(f"  - {attr}")
