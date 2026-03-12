#!/usr/bin/env python3
"""
SS v3.0 量化投资框架 - 回测引擎
支持历史回测、信号验证、绩效分析
"""

import pandas as pd
import numpy as np
from datetime import datetime, timedelta
from typing import Dict, List, Tuple
import json
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from backtest.data_source import DataManager
from backtest.scoring import SSScoring


class BacktestEngine:
    """
    回测引擎
    
    策略逻辑:
    1. 每日扫描候选股票池
    2. 计算 SS 评分
    3. 满足 BUY 信号且档位 A/B → 建仓
    4. 满足 SELL 信号或破位 → 清仓
    5. 否则 HOLD
    """
    
    def __init__(self, 
                 initial_capital: float = 1000000,
                 commission: float = 0.0003,
                 slippage: float = 0.001):
        """
        Args:
            initial_capital: 初始资金 (默认100万)
            commission: 手续费率 (默认万三)
            slippage: 滑点 (默认千一)
        """
        self.initial_capital = initial_capital
        self.commission = commission
        self.slippage = slippage
        
        self.dm = DataManager()
        self.scorer = SSScoring(self.dm)
        
        self.positions = {}
        self.cash = initial_capital
        self.history = []
        self.daily_equity = []
        
    def run(self, 
            symbols: List[str], 
            start_date: str, 
            end_date: str,
            rebalance_freq: int = 5) -> Dict:
        """
        运行回测
        """
        print(f"\n{'='*60}")
        print(f"SS v3.0 回测系统")
        print(f"{'='*60}")
        print(f"股票池: {len(symbols)} 只")
        print(f"回测期: {start_date} ~ {end_date}")
        print(f"初始资金: {self.initial_capital:,.0f} 元")
        
        start = datetime.strptime(start_date, "%Y%m%d")
        end = datetime.strptime(end_date, "%Y%m%d")
        trade_dates = pd.date_range(start, end, freq='B')
        
        print(f"交易日数: {len(trade_dates)} 天")
        
        for i, date in enumerate(trade_dates):
            date_str = date.strftime("%Y%m%d")
            
            if i % rebalance_freq == 0:
                signals = self._scan_signals(symbols, date_str)
                self._execute_signals(signals, date_str)
            
            self._update_equity(date_str, symbols)
        
        result = self._calc_performance()
        
        return result
    
    def _scan_signals(self, symbols: List[str], date: str) -> List[Dict]:
        """扫描信号"""
        signals = []
        
        for symbol in symbols:
            try:
                result = self.scorer.calc_all(symbol, date)
                
                if result['signal'] == 'BUY' and result['rank'] in ['A', 'B']:
                    signals.append({
                        'symbol': symbol,
                        'action': 'BUY',
                        'rank': result['rank'],
                        'score': result['total_score'],
                        'price': self._get_price(symbol, date)
                    })
            except Exception as e:
                continue
        
        signals.sort(key=lambda x: x['score'], reverse=True)
        
        return signals
    
    def _execute_signals(self, signals: List[Dict], date: str):
        """执行信号"""
        for symbol in list(self.positions.keys()):
            if self._should_sell(symbol, date):
                self._sell(symbol, date)
        
        available_capital = self.cash
        max_positions = 5
        current_positions = len(self.positions)
        
        for signal in signals:
            if current_positions >= max_positions:
                break
            
            if available_capital <= 0:
                break
            
            symbol = signal['symbol']
            if symbol in self.positions:
                continue
            
            price = signal['price']
            if price <= 0:
                continue
            
            target_amount = available_capital * 0.2
            shares = int(target_amount / price / 100) * 100
            
            if shares > 0:
                self._buy(symbol, date, price, shares)
                current_positions += 1
                available_capital -= shares * price
    
    def _should_sell(self, symbol: str, date: str) -> bool:
        """判断是否应该卖出"""
        try:
            result = self.scorer.calc_all(symbol, date)
            
            if result['signal'] == 'SELL':
                return True
            
            if result['rank'] == 'D':
                return True
            
            if symbol in self.positions:
                cost = self.positions[symbol]['cost']
                price = self._get_price(symbol, date)
                if price < cost * 0.9:
                    return True
            
            return False
        except:
            return False
    
    def _buy(self, symbol: str, date: str, price: float, shares: int):
        """买入"""
        buy_price = price * (1 + self.slippage)
        
        commission = shares * buy_price * self.commission
        total_cost = shares * buy_price + commission
        
        if total_cost > self.cash:
            return
        
        self.cash -= total_cost
        self.positions[symbol] = {
            'shares': shares,
            'cost': buy_price,
            'buy_date': date
        }
        
        self.history.append({
            'date': date,
            'symbol': symbol,
            'action': 'BUY',
            'price': buy_price,
            'shares': shares,
            'amount': total_cost
        })
    
    def _sell(self, symbol: str, date: str):
        """卖出"""
        if symbol not in self.positions:
            return
        
        position = self.positions[symbol]
        shares = position['shares']
        price = self._get_price(symbol, date)
        
        sell_price = price * (1 - self.slippage)
        
        commission = shares * sell_price * self.commission
        total_proceeds = shares * sell_price - commission
        
        self.cash += total_proceeds
        
        cost = position['cost'] * shares
        profit = total_proceeds - cost
        profit_pct = profit / cost * 100
        
        self.history.append({
            'date': date,
            'symbol': symbol,
            'action': 'SELL',
            'price': sell_price,
            'shares': shares,
            'amount': total_proceeds,
            'profit': profit,
            'profit_pct': profit_pct
        })
        
        del self.positions[symbol]
    
    def _get_price(self, symbol: str, date: str) -> float:
        """获取收盘价"""
        try:
            date_fmt = datetime.strptime(date, "%Y%m%d")
            start = (date_fmt - timedelta(days=30)).strftime("%Y%m%d")
            end = date
            
            daily = self.dm.get_daily(symbol, start, end)
            if daily is not None and len(daily) > 0:
                return daily['close'].iloc[-1]
        except:
            pass
        return 0
    
    def _update_equity(self, date: str, symbols: List[str]):
        """更新每日权益"""
        position_value = 0
        for symbol, pos in self.positions.items():
            price = self._get_price(symbol, date)
            if price > 0:
                position_value += pos['shares'] * price
        
        total_equity = self.cash + position_value
        
        self.daily_equity.append({
            'date': date,
            'cash': self.cash,
            'position_value': position_value,
            'total_equity': total_equity,
            'positions': len(self.positions)
        })
    
    def _calc_performance(self) -> Dict:
        """计算绩效指标"""
        if not self.daily_equity:
            return {}
        
        equity_df = pd.DataFrame(self.daily_equity)
        
        equity_df['return'] = equity_df['total_equity'].pct_change()
        
        total_return = (equity_df['total_equity'].iloc[-1] / 
                       equity_df['total_equity'].iloc[0] - 1) * 100
        
        days = len(equity_df)
        annual_return = total_return * 252 / days if days > 0 else 0
        
        equity_df['cummax'] = equity_df['total_equity'].cummax()
        equity_df['drawdown'] = (equity_df['total_equity'] / 
                                equity_df['cummax'] - 1) * 100
        max_drawdown = equity_df['drawdown'].min()
        
        daily_returns = equity_df['return'].dropna()
        if len(daily_returns) > 0 and daily_returns.std() > 0:
            sharpe = daily_returns.mean() / daily_returns.std() * np.sqrt(252)
        else:
            sharpe = 0
        
        trades = [t for t in self.history if t['action'] == 'SELL']
        if trades:
            wins = [t for t in trades if t.get('profit', 0) > 0]
            win_rate = len(wins) / len(trades) * 100
        else:
            win_rate = 0
        
        if trades:
            avg_win = np.mean([t['profit'] for t in trades if t.get('profit', 0) > 0]) if wins else 0
            avg_loss = abs(np.mean([t['profit'] for t in trades if t.get('profit', 0) < 0])) if [t for t in trades if t.get('profit', 0) < 0] else 1
            profit_loss_ratio = avg_win / avg_loss if avg_loss > 0 else 0
        else:
            profit_loss_ratio = 0
        
        result = {
            'initial_capital': self.initial_capital,
            'final_equity': equity_df['total_equity'].iloc[-1],
            'total_return': total_return,
            'annual_return': annual_return,
            'max_drawdown': max_drawdown,
            'sharpe_ratio': sharpe,
            'win_rate': win_rate,
            'profit_loss_ratio': profit_loss_ratio,
            'total_trades': len(trades),
            'trading_days': days,
            'equity_curve': equity_df.to_dict('records'),
            'trades': self.history
        }
        
        return result
    
    def print_result(self, result: Dict):
        """打印回测结果"""
        print("\n" + "=" * 60)
        print("               回测结果报告")
        print("=" * 60)
        
        print(f"\n【收益概况】")
        print(f"  初始资金:      {result['initial_capital']:>12,.0f} 元")
        print(f"  最终权益:      {result['final_equity']:>12,.0f} 元")
        print(f"  总收益率:      {result['total_return']:>12.2f} %")
        print(f"  年化收益率:    {result['annual_return']:>12.2f} %")
        
        print(f"\n【风险指标】")
        print(f"  最大回撤:      {result['max_drawdown']:>12.2f} %")
        print(f"  夏普比率:      {result['sharpe_ratio']:>12.2f}")
        
        print(f"\n【交易统计】")
        print(f"  总交易次数:    {result['total_trades']:>12} 次")
        print(f"  胜率:          {result['win_rate']:>12.2f} %")
        print(f"  盈亏比:        {result['profit_loss_ratio']:>12.2f}")
        print(f"  交易天数:      {result['trading_days']:>12} 天")
        
        if result['trades']:
            print(f"\n【交易明细】")
            for i, trade in enumerate(result['trades'][-10:], 1):
                if trade['action'] == 'BUY':
                    print(f"  {i}. {trade['date']} 买入 {trade['symbol']} "
                          f"{trade['shares']}股 @ {trade['price']:.2f}")
                else:
                    print(f"  {i}. {trade['date']} 卖出 {trade['symbol']} "
                          f"{trade['shares']}股 @ {trade['price']:.2f} "
                          f"盈亏: {trade.get('profit_pct', 0):.2f}%")
        
        print("\n" + "=" * 60)


if __name__ == "__main__":
    symbols = [
        "688008",
        "600519",
        "000858",
        "300750",
        "002594",
        "600036",
        "000333",
        "601318",
        "000001",
        "600900",
    ]
    
    engine = BacktestEngine(
        initial_capital=1000000,
        commission=0.0003,
        slippage=0.001
    )
    
    result = engine.run(
        symbols=symbols,
        start_date="20240101",
        end_date="20240309",
        rebalance_freq=5
    )
    
    engine.print_result(result)
