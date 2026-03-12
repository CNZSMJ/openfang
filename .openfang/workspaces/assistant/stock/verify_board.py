import sys
sys.path.insert(0, '/Users/huangjiahao/.openfang/workspaces/assistant/stock')
import akshare as ak
from eastmoney_money_flow import get_board_money_flow

date = '20260310'

# 1. 涨停板板块
df_zt = ak.stock_zt_pool_em(date=date)
zt_boards = set(df_zt['所属行业'].unique())
print("=== 涨停板板块(akshare-所属行业) ===")
print(sorted(zt_boards))

# 2. 资金流向板块
mf = get_board_money_flow(30)
mf_boards = set([item['板块名'] for item in mf])
print("\n=== 资金流向板块(东财API) ===")
print(sorted(mf_boards))

# 3. 交集
intersection = zt_boards & mf_boards
print(f"\n=== 交集(能匹配的) ===")
print(sorted(intersection))

# 4. 差集
zt_only = zt_boards - mf_boards
mf_only = mf_boards - zt_boards
print(f"\n=== 涨停板有、资金没有 ===")
print(sorted(zt_only))
print(f"\n=== 资金有、涨停板没有 ===")
print(sorted(mf_only))
