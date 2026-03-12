import akshare as ak
import pandas as pd

# 获取所有A股代码和名称
# 然后获取实时数据来计算涨跌比

# 方法：获取沪深指数的涨跌来估算
# 或者用概念指数来反推

# 试试能不能获取市场概况
try:
    df = ak.stock_fund_flow()
    print("资金流列:", df.columns.tolist())
    print(df.head(3))
except Exception as e:
    print(f"error: {e}")

# 试试行业板块
try:
    df = ak.stock_board_industry_name_em()
    print("\n行业板块:", df.columns.tolist())
    print(df.head(3))
except Exception as e:
    print(f"error: {e}")
