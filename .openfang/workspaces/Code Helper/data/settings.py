# -*- coding: utf-8 -*-
"""SS v3.0 量化回测框架 - 配置模块"""

# ============== 回测参数 ==============
BACKTEST_CONFIG = {
    'start_date': '20200101',
    'end_date': '20231231',
    'initial_capital': 1_000_000,
    'commission': 0.0003,
    'stamp_duty': 0.001,
    'slippage': 0.001,
    'rebalance_freq': 5,
}

# ============== 评分参数 ==============
SCORE_CONFIG = {
    'value_weight': 0.35,
    'trade_weight': 0.40,
    'risk_weight': 0.25,
    'value_sub': {
        'fundamental': 0.40,
        'industry': 0.30,
        'valuation': 0.30,
    },
    'trade_sub': {
        'fund_structure': 0.35,
        'k_pattern': 0.35,
        'volume_price': 0.30,
    },
    'risk_sub': {
        'volatility': 0.35,
        'event': 0.35,
        'liquidity': 0.30,
    },
    'tier_thresholds': {'A+': 90, 'A': 80, 'B': 70, 'C': 60, 'D': 0},
}

PORTFOLIO_CONFIG = {
    'tier_position': {'A+': 0.20, 'A': 0.15, 'B': 0.10, 'C': 0.05, 'D': 0.00},
    'market_env_factor': {'bull': 1.2, 'bear': 0.5, '震荡': 0.8},
    'max_positions': 10,
    'max_single_position': 0.25,
    'min_position': 0.30,
}

SIGNAL_CONFIG = {
    'buy_threshold': 80,
    'watch_threshold': 70,
    'sell_threshold': 60,
    'hold_threshold': 65,
}
