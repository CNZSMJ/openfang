#!/usr/bin/env python3
"""
SS v4.0 回测引擎 - 支持历史时点财务数据
"""

import pandas as pd
import numpy as np
from datetime import datetime, timedelta
from collections import defaultdict
from data_source_v4 import DataManagerV4
from scoring_v4 import ScoringV4


class BacktestEngineV4:
    """
    回测引擎 v4.0
    
    核心改进:
    - 每次调仓使用历史时点的财务数据 (解决look-ahead bias)
    - 模拟真实交易成本
    - 完整的风险控制
    """
    
    def __init__(self, dm: DataManagerV4, scorer: ScoringV4):
        self.dm = dm
        self.scorer = scorer
        
        # 回测参数
        self.initial_capital = 1000000  # 初始资金100万
        self.rebalance_days = 20        # 调仓周期20天
        self.max_positions = 10          # 最大持仓数
        self.stop_loss_pct = 0.08        # 8%止损
        self.take_profit_pct = 0.20     # 20%止盈
        
        # 交易成本
        self.commission_rate = 0.0003   # 万三佣金
        self.slippage = 0.001            # 千一滑点
        self.stamp_duty = 0.001          # 千一印花税 (卖出)
        
        # 状态
        self.positions = {}  # {symbol: {'shares': int, 'cost': float}}
        self.cash = self.initial_capital  # 现金
        self.portfolio_value = self.initial_capital
        self.trade_history = []
        self.daily_values = []
        self.last_trade_date = None  # 最后一个交易日
    
    def run(self, symbols, start_date, end_date):
        """
        运行回测
        """
        print(f"\n{'='*60}")
        print(f"SS v4.0 回测引擎")
        print(f"{'='*60}")
        print(f"股票池: {len(symbols)} 只")
        print(f"回测期: {start_date} ~ {end_date}")
        print(f"初始资金: {self.initial_capital/10000:.0f}万")
        print(f"调仓周期: {self.rebalance_days}天")
        
        # 生成交易日期序列
        trade_dates = self._generate_trade_dates(start_date, end_date)
        
        if len(trade_dates) < self.rebalance_days:
            print("回测期太短")
            return None
        
        last_rebalance_idx = -1
        
        for i, date in enumerate(trade_dates):
            # 格式化为 YYYYMMDD 字符串
            if isinstance(date, datetime):
                date_str = date.strftime('%Y%m%d')
            else:
                date_str = str(date)
            
            self.last_trade_date = date_str
            
            # 获取当日持仓市值
            self._update_portfolio_value(date_str)
            
            # 记录每日净值
            self.daily_values.append({
                'date': date_str,
                'portfolio_value': self.portfolio_value,
                'cash': self.cash,
                'positions_value': self.portfolio_value - self.cash,
                'positions': len(self.positions),
            })
            
            # 调仓检查 (每rebalance_days天 or 首次)
            if i == 0 or (i - last_rebalance_idx) >= self.rebalance_days:
                print(f"\n[{date_str}] 调仓 (现金: {self.cash/10000:.2f}万, 持仓: {(self.portfolio_value-self.cash)/10000:.2f}万)")
                
                # 1. 获取评分 (使用历史时点财务数据!)
                ranked = self.scorer.rank_stocks(symbols, date_str, top_n=self.max_positions)
                
                if not ranked:
                    print(f"  无评分数据，跳过")
                    continue
                
                # 2. 确定目标持仓
                target_symbols = [r['symbol'] for r in ranked[:self.max_positions]]
                
                # 3. 执行调仓
                self._rebalance(target_symbols, date_str)
                
                last_rebalance_idx = i
            
            # 4. 止损止盈检查
            self._check_stop_loss(date_str)
        
        # 回测结束 - 使用最后一个交易日计算最终市值
        if self.last_trade_date:
            self._update_portfolio_value(self.last_trade_date)
        
        # 输出结果
        return self._generate_report()
    
    def _generate_trade_dates(self, start_date, end_date):
        """生成交易日期序列"""
        all_dates = set()
        
        for symbol in ['600519', '000858', '600036']:
            df = self.dm.get_daily(symbol, start_date, end_date)
            if df is not None:
                all_dates.update(df['日期'].dt.strftime('%Y%m%d').tolist())
        
        dates = sorted([d for d in all_dates if start_date <= d <= end_date])
        return [datetime.strptime(d, '%Y%m%d') for d in dates]
    
    def _update_portfolio_value(self, date_str):
        """更新组合市值 = 现金 + 持仓市值"""
        # 计算持仓市值
        positions_value = 0
        for symbol, pos in self.positions.items():
            price = self._get_price_on_date(symbol, date_str)
            if price:
                positions_value += pos['shares'] * price
        
        self.portfolio_value = self.cash + positions_value
    
    def _get_price_on_date(self, symbol, date_str):
        """获取指定日期的价格"""
        df = self.dm.get_daily(symbol, date_str, date_str)
        if df is not None and len(df) > 0:
            return df.iloc[0]['收盘']
        return None
    
    def _rebalance(self, target_symbols, date_str):
        """执行调仓"""
        current_symbols = set(self.positions.keys())
        target_set = set(target_symbols)
        
        # 卖出不在目标中的
        to_sell = current_symbols - target_set
        for symbol in to_sell:
            self._sell_all(symbol, date_str)
        
        # 买入新标的
        to_buy = target_set - current_symbols
        if to_buy and self.cash > 0:
            # 计算每只股票的分配金额
            per_stock = self.cash / len(to_buy) * 0.95  # 预留5%缓冲
            
            for symbol in to_buy:
                self._buy(symbol, per_stock, date_str)
        
        print(f"  持仓: {list(self.positions.keys())}")
    
    def _buy(self, symbol, amount, date_str):
        """买入"""
        price = self._get_price_on_date(symbol, date_str)
        if price is None:
            return
        
        # 考虑滑点
        buy_price = price * (1 + self.slippage)
        
        # 计算可买数量
        available = self.cash / (1 + self.slippage) / (1 + self.commission_rate)
        shares = int(min(amount, available) / buy_price / 100) * 100  # 整手
        
        if shares < 100:
            return
        
        # 计算成本
        cost = shares * buy_price
        commission = cost * self.commission_rate
        total_cost = cost + commission
        
        # 扣减现金
        self.cash -= total_cost
        
        # 记录持仓
        self.positions[symbol] = {
            'shares': shares,
            'cost': buy_price,
            'buy_date': date_str,
        }
        
        # 记录交易
        self.trade_history.append({
            'date': date_str,
            'symbol': symbol,
            'action': 'BUY',
            'price': buy_price,
            'shares': shares,
            'cost': total_cost,
            'commission': commission,
        })
    
    def _sell_all(self, symbol, date_str):
        """清仓卖出"""
        if symbol not in self.positions:
            return
        
        pos = self.positions[symbol]
        price = self._get_price_on_date(symbol, date_str)
        
        if price is None:
            return
        
        # 考虑滑点和印花税
        sell_price = price * (1 - self.slippage - self.stamp_duty)
        
        proceeds = pos['shares'] * sell_price
        commission = proceeds * self.commission_rate
        stamp_duty = pos['shares'] * price * self.stamp_duty
        total_proceeds = proceeds - commission - stamp_duty
        
        # 增加现金
        self.cash += total_proceeds
        
        # 记录交易
        self.trade_history.append({
            'date': date_str,
            'symbol': symbol,
            'action': 'SELL',
            'price': sell_price,
            'shares': pos['shares'],
            'proceeds': total_proceeds,
            'commission': commission,
            'stamp_duty': stamp_duty,
            'pnl': proceeds - pos['shares'] * pos['cost'],
        })
        
        del self.positions[symbol]
    
    def _check_stop_loss(self, date_str):
        """检查止损止盈"""
        to_close = []
        
        for symbol, pos in self.positions.items():
            price = self._get_price_on_date(symbol, date_str)
            if price is None:
                continue
            
            pct_change = (price - pos['cost']) / pos['cost']
            
            # 止损
            if pct_change <= -self.stop_loss_pct:
                to_close.append(symbol)
                print(f"  止损: {symbol} {pct_change*100:.1f}%")
            # 止盈
            elif pct_change >= self.take_profit_pct:
                to_close.append(symbol)
                print(f"  止盈: {symbol} {pct_change*100:.1f}%")
        
        for symbol in to_close:
            self._sell_all(symbol, date_str)
    
    def _generate_report(self):
        """生成回测报告"""
        df = pd.DataFrame(self.daily_values)
        
        if len(df) == 0:
            return None
        
        # 计算收益率
        df['return'] = df['portfolio_value'] / self.initial_capital - 1
        
        # 年化收益率
        days = len(df)
        years = days / 252
        total_return = df.iloc[-1]['return']
        annual_return = (1 + total_return) ** (1 / years) - 1 if years > 0 else 0
        
        # 最大回撤
        df['peak'] = df['portfolio_value'].cummax()
        df['drawdown'] = (df['portfolio_value'] - df['peak']) / df['peak']
        max_drawdown = df['drawdown'].min()
        
        # 夏普比率
        daily_returns = df['return'].diff().dropna()
        sharpe = daily_returns.mean() / daily_returns.std() * np.sqrt(252) if daily_returns.std() > 0 else 0
        
        # 交易统计
        trades = pd.DataFrame(self.trade_history)
        num_trades = len(trades) if len(trades) > 0 else 0
        
        print(f"\n{'='*60}")
        print(f"回测结果")
        print(f"{'='*60}")
        print(f"总收益率: {total_return*100:.2f}%")
        print(f"年化收益率: {annual_return*100:.2f}%")
        print(f"最大回撤: {max_drawdown*100:.2f}%")
        print(f"夏普比率: {sharpe:.2f}")
        print(f"交易次数: {num_trades}")
        print(f"最终市值: {self.portfolio_value/10000:.2f}万")
        print(f"最终现金: {self.cash/10000:.2f}万")
        
        return {
            'total_return': total_return,
            'annual_return': annual_return,
            'max_drawdown': max_drawdown,
            'sharpe': sharpe,
            'num_trades': num_trades,
            'final_value': self.portfolio_value,
            'final_cash': self.cash,
            'daily_df': df,
            'trades': trades,
        }


# 测试
if __name__ == "__main__":
    dm = DataManagerV4()
    scorer = ScoringV4(dm)
    engine = BacktestEngineV4(dm, scorer)
    
    # 简化股票池 (10只)
    symbols = [
        "600519",  # 茅台
        "000858",  # 五粮液
        "600036",  # 招商银行
        "601318",  # 中国平安
        "000333",  # 美的
        "600887",  # 伊利
        "000002",  # 万科
        "600030",  # 中信证券
        "601888",  # 中国中免
        "600900",  # 长江电力
    ]
    
    # 2025年回测
    result = engine.run(symbols, "20250101", "20251231")
    
    print("\n✓ 回测完成!")
