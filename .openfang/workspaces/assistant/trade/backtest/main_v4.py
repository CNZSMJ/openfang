#!/usr/bin/env python3
"""
SS v4.0 主程序 - 快速测试
预加载所有数据，避免频繁网络请求
"""

import sys
import os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from config import REBALANCE_FREQ
from data_source_v4 import DataManagerV4
from scoring_v4 import ScoringV4
from engine_v4 import BacktestEngineV4


def preload_all_data(dm, symbols, start_date, end_date):
    """预加载所有股票的所有历史数据"""
    print("\n【预加载数据】")
    
    # 预加载日线数据
    for symbol in symbols:
        print(f"  加载 {symbol}...")
        dm.get_daily(symbol, start_date, end_date)
    
    print(f"  ✓ 完成，共 {len(symbols)} 只股票")


def main():
    print("\n" + "=" * 60)
    print("        SS v4.0 量化投资框架 - 快速测试")
    print("=" * 60)
    
    # 小规模测试
    symbols = ["600519", "000858", "600036", "601318", "600900", 
               "000333", "002594", "300750", "601012", "600016"]
    
    # 缩短回测期，减少请求
    start_date = "20231201"
    end_date = "20231231"
    initial_capital = 1000000
    
    # ==================== 初始化 ====================
    
    print("\n【1. 初始化数据源】")
    dm = DataManagerV4()
    
    # 预加载数据
    preload_all_data(dm, symbols, start_date, end_date)
    
    print("\n【2. 初始化评分器】")
    scorer = ScoringV4(dm)
    
    # ==================== 测试评分 ====================
    
    print("\n【3. 测试评分系统】")
    print("评估日期: 2023年12月31日")
    
    results_df = scorer.rank_stocks(symbols, "20231231", top_n=10)
    
    if not results_df.empty:
        print("\n评分结果:")
        print(f"{'代码':<8} {'总分':>6} {'价值':>6} {'动量':>6} {'质量':>6} {'信号':<6}")
        print("-" * 50)
        for _, r in results_df.iterrows():
            print(f"{r['symbol']:<8} {r['total_score']:>6.1f} {r['value_score']:>6.1f} {r['momentum_score']:>6.1f} {r['quality_score']:>6.1f} {r['signal']:<6}")
    
    # ==================== 运行回测 ====================
    
    print("\n【4. 运行回测】")
    print(f"回测期: {start_date} ~ {end_date}")
    
    # 传入dm给engine复用
    engine = BacktestEngineV4(
        initial_capital=initial_capital,
        commission=0.0003,
        slippage=0.001,
        data_manager=dm  # 复用数据管理器
    )
    
    result = engine.run(
        symbols=symbols,
        start_date=start_date,
        end_date=end_date,
        scorer=scorer,
        rebalance_freq=5
    )
    
    # ==================== 输出结果 ====================
    
    engine.print_result(result)
    
    print("\n" + "=" * 60)


if __name__ == "__main__":
    main()
