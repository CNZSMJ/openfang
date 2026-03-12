import sys
sys.path.insert(0, '/Users/huangjiahao/.openfang/workspaces/assistant/stock')
from eastmoney_money_flow import get_board_money_flow

# 获取资金流向
mf = get_board_money_flow(20)
print("=== 资金流向板块 ===")
for item in mf[:10]:
    print(f"{item['板块名']}: {item['主力净流入最大股']}")
