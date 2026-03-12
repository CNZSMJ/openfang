#!/usr/bin/env python3
"""
SS v3.0 量化投资框架 - 主程序
运行回测并生成报告
"""

import sys
import os

# 添加当前目录到路径
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from data_source import DataManager
from scoring import SSScoring
from engine import BacktestEngine


def main():
    """主函数"""
    print("\n" + "=" * 60)
    print("        SS v3.0 量化投资框架 - 回测系统")
    print("=" * 60)
    
    # ==================== 配置 ====================
    
    # 测试股票池 (A股核心标的)
    symbols = [
        "688008",  # 澜起科技 - 半导体
        "600519",  # 贵州茅台 - 消费
        "300750",  # 宁德时代 - 新能源
        "002594",  # 比亚迪 - 新能源车
        "600036",  # 招商银行 - 金融
        "000333",  # 美的集团 - 家电
        "601318",  # 中国平安 - 保险
        "600900",  # 长江电力 - 公用事业
        "000858",  # 五粮液 - 白酒
        "300059",  # 东方财富 - 互联网金融
    ]
    
    # 回测参数
    start_date = "20240101"
    end_date = "20240309"
    initial_capital = 1000000  # 100万
    
    # ==================== 第一步：数据源测试 ====================
    
    print("\n【第一步】数据源初始化...")
    dm = DataManager()
    
    # ==================== 第二步：单票评分测试 ====================
    
    print("\n【第二步】单票评分测试 (澜起科技 688008)...")
    scorer = SSScoring(dm)
    result = scorer.calc_all("688008")
    scorer.print_report(result)
    
    # ==================== 第三步：运行回测 ====================
    
    print("\n【第三步】运行回测...")
    engine = BacktestEngine(
        initial_capital=initial_capital,
        commission=0.0003,  # 万三手续费
        slippage=0.001    # 千一滑点
    )
    
    result = engine.run(
        symbols=symbols,
        start_date=start_date,
        end_date=end_date,
        rebalance_freq=5  # 每5天调仓
    )
    
    # ==================== 第四步：输出结果 ====================
    
    print("\n【第四步】回测结果")
    engine.print_result(result)
    
    # ==================== 第五步：详细分析 ====================
    
    print("\n【第五步】绩效分析")
    
    if result['total_return'] > 0:
        print(f"  ✓ 策略盈利 {result['total_return']:.2f}%")
    else:
        print(f"  ✗ 策略亏损 {abs(result['total_return']):.2f}%")
    
    if result['sharpe_ratio'] > 1:
        print(f"  ✓ 夏普比率 {result['sharpe_ratio']:.2f} (优秀)")
    elif result['sharpe_ratio'] > 0.5:
        print(f"  ○ 夏普比率 {result['sharpe_ratio']:.2f} (一般)")
    else:
        print(f"  ✗ 夏普比率 {result['sharpe_ratio']:.2f} (较差)")
    
    if result['max_drawdown'] > -20:
        print(f"  ✓ 最大回撤 {result['max_drawdown']:.2f}% (可控)")
    else:
        print(f"  ✗ 最大回撤 {result['max_drawdown']:.2f}% (过大)")
    
    if result['win_rate'] > 50:
        print(f"  ✓ 胜率 {result['win_rate']:.2f}% (盈利)")
    else:
        print(f"  ✗ 胜率 {result['win_rate']:.2f}% (亏损)")
    
    print("\n" + "=" * 60)
    print("                    回测完成")
    print("=" * 60)


if __name__ == "__main__":
    main()
