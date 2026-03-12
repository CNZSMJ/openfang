#!/usr/bin/env python3
"""
SS v4.0 回测引擎 - 模拟实盘交易
"""

import pandas as pd
import numpy as np
from datetime import datetime, timedelta
from config import (
    DEFAULT_INITIAL_CAPITAL, COMMISSION_RATE, SLIPPAGE_RATE, STAMP_TAX_RATE,
    MAX_POSITIONS, MAX_POSITION_SIZE, MIN_POSITION_SIZE,
    REBALANCE_FREQ, STOP_LOSS_PCT, TAKE_PROFIT_PCT
)


class BacktestEngineV4:
    """
    回测引擎 v4.0
    """
    
    def __init__(self, initial_capital=DEFAULT_INITIAL_CAPITAL,
                 commission=COMMISSION_RATE,
                 slippage=SLIPPAGE_RATE,
                 stamp_tax=STAMP_TAX_RATE,
                 data_manager=None):
        self.initial_capital = initial_capital
        self.commission = commission
        self.slippage = slippage
        self.stamp_tax = stamp_tax
        
        # 数据管理器（复用）
        self.dm = data_manager
        
        # 持仓状态
        self.positions = {}
        self.cash = initial_capital
        self.equity_curve = []
        self.trades = []
        self.daily_stats = []
    
    def run(self, symbols, start_date, end_date, scorer, rebalance_freq=REBALANCE_FREQ):
        """运行回测"""
        print(f"\n{'='*60}")
        print(f"SS v4.0 回测系统")
        print(f"{'='*60}")
        print(f"股票池: {len(symbols)} 只")
        print(f"回测期: {start_date} ~ {end_date}")
        print(f"初始资金: {self.initial_capital:,} 元")
        print(f"调仓频率: 每 {rebalance_freq} 天")
        
        trading_days = self._get_trading_days(start_date, end_date)
        print(f"交易日数: {len(trading_days)} 天")
        
        for i, day in enumerate(trading_days):
            day_str = day.strftime("%Y%m%d")
            
            prices = self._get_current_prices(symbols, day_str)
            if not prices:
                continue
            
            should_rebalance = (i % rebalance_freq == 0) and (i > 0)
            
            if should_rebalance:
                self._rebalance(symbols, day_str, prices, scorer)
            
            self._check_stop_loss(prices)
            self._update_equity(day_str, prices)
        
        result = self._generate_result()
        return result
    
    def _get_trading_days(self, start_date, end_date):
        """生成交易日列表"""
        start = datetime.strptime(start_date, "%Y%m%d")
        end = datetime.strptime(end_date, "%Y%m%d")
        
        days = []
        current = start
        while current <= end:
            if current.weekday() < 5:
                days.append(current)
            current += timedelta(days=1)
        return days
    
    def _get_current_prices(self, symbols, date):
        """获取当日收盘价 - 复用dm"""
        prices = {}
        for symbol in symbols:
            try:
                df = self.dm.get_daily(symbol, date, date)
                if df is not None and len(df) > 0:
                    prices[symbol] = df.iloc[0]['收盘']
            except:
                pass
        return prices
    
    def _rebalance(self, symbols, date, prices, scorer):
        """执行调仓"""
        ranked = scorer.rank_stocks(symbols, date, top_n=MAX_POSITIONS * 2)
        
        if ranked.empty:
            return
        
        buy_candidates = ranked[ranked['signal'] == 'BUY'].head(MAX_POSITIONS)
        target_positions = set(buy_candidates['symbol'].tolist())
        
        # 卖出不在目标持仓的
        to_sell = [s for s in self.positions.keys() if s not in target_positions]
        for symbol in to_sell:
            self._sell_stock(symbol, date, prices.get(symbol), "调仓卖出")
        
        # 买入新股票
        current_positions = set(self.positions.keys())
        available_cash = self.cash
        positions_to_buy = target_positions - current_positions
        
        num_to_buy = len(positions_to_buy)
        if num_to_buy > 0:
            target_size = min(MAX_POSITION_SIZE, 1.0 / num_to_buy)
            
            for symbol in positions_to_buy:
                if available_cash <= 0:
                    break
                
                price = prices.get(symbol)
                if price is None:
                    continue
                
                max_shares = int(available_cash * target_size / price / 100) * 100
                
                if max_shares > 0:
                    self._buy_stock(symbol, date, price, max_shares)
                    available_cash = self.cash
    
    def _buy_stock(self, symbol, date, price, shares):
        """买入股票"""
        actual_price = price * (1 + self.slippage)
        amount = actual_price * shares
        commission = amount * self.commission
        
        if self.cash >= amount + commission:
            self.cash -= (amount + commission)
            self.positions[symbol] = {
                'shares': shares,
                'cost': actual_price,
                'buy_date': date
            }
            
            self.trades.append({
                'date': date,
                'symbol': symbol,
                'action': 'BUY',
                'price': actual_price,
                'shares': shares,
                'amount': amount,
                'commission': commission
            })
            
            print(f"  买入 {symbol} {shares}股 @ {actual_price:.2f}")
    
    def _sell_stock(self, symbol, date, price, reason):
        """卖出股票"""
        if symbol not in self.positions:
            return
        
        pos = self.positions[symbol]
        shares = pos['shares']
        
        actual_price = price * (1 - self.slippage)
        amount = actual_price * shares
        commission = amount * self.commission
        tax = amount * self.stamp_tax
        profit = (actual_price - pos['cost']) * shares - commission - tax
        profit_pct = (actual_price - pos['cost']) / pos['cost'] * 100
        
        self.cash += (amount - commission - tax)
        
        self.trades.append({
            'date': date,
            'symbol': symbol,
            'action': 'SELL',
            'price': actual_price,
            'shares': shares,
            'amount': amount,
            'commission': commission,
            'tax': tax,
            'profit': profit,
            'profit_pct': profit_pct,
            'reason': reason
        })
        
        print(f"  卖出 {symbol} {shares}股 @ {actual_price:.2f} ({profit_pct:+.2f}%) {reason}")
        
        del self.positions[symbol]
    
    def _check_stop_loss(self, prices):
        """检查止损止盈"""
        to_sell = []
        
        for symbol, pos in self.positions.items():
            price = prices.get(symbol)
            if price is None:
                continue
            
            change_pct = (price - pos['cost']) / pos['cost']
            
            if change_pct <= STOP_LOSS_PCT:
                to_sell.append((symbol, "止损"))
            elif change_pct >= TAKE_PROFIT_PCT:
                to_sell.append((symbol, "止盈"))
        
        for symbol, reason in to_sell:
            self._sell_stock(symbol, datetime.now().strftime("%Y%m%d"), 
                           prices.get(symbol), reason)
    
    def _update_equity(self, date, prices):
        """更新权益曲线"""
        position_value = 0
        for symbol, pos in self.positions.items():
            price = prices.get(symbol, pos['cost'])
            position_value += pos['shares'] * price
        
        total_equity = self.cash + position_value
        
        self.equity_curve.append({
            'date': date,
            'cash': self.cash,
            'position_value': position_value,
            'total_equity': total_equity,
            'positions': len(self.positions)
        })
        
        self.daily_stats.append({
            'date': date,
            'total_equity': total_equity
        })
    
    def _generate_result(self):
        """生成回测结果"""
        if not self.equity_curve:
            return {}
        
        df = pd.DataFrame(self.equity_curve)
        
        initial = self.initial_capital
        final = df.iloc[-1]['total_equity']
        total_return = (final - initial) / initial * 100
        
        days = len(df)
        annual_return = total_return * 252 / days
        
        df['cummax'] = df['total_equity'].cummax()
        df['drawdown'] = (df['total_equity'] - df['cummax']) / df['cummax'] * 100
        max_drawdown = df['drawdown'].min()
        
        daily_returns = df['total_equity'].pct_change().dropna()
        if len(daily_returns) > 0 and daily_returns.std() > 0:
            sharpe = daily_returns.mean() / daily_returns.std() * np.sqrt(252)
        else:
            sharpe = 0
        
        trades_df = pd.DataFrame(self.trades)
        if len(trades_df) > 0:
            sells = trades_df[trades_df['action'] == 'SELL']
            if len(sells) > 0:
                wins = sells[sells['profit'] > 0]
                win_rate = len(wins) / len(sells) * 100
                avg_profit = sells[sells['profit'] > 0]['profit'].mean() if len(wins) > 0 else 0
                avg_loss = sells[sells['profit'] < 0]['profit'].mean() if len(sells) > len(wins) else 0
                profit_loss_ratio = abs(avg_profit / avg_loss) if avg_loss != 0 else 0
            else:
                win_rate = 0
                profit_loss_ratio = 0
        else:
            win_rate = 0
            profit_loss_ratio = 0
        
        return {
            'initial_capital': initial,
            'final_equity': final,
            'total_return': total_return,
            'annual_return': annual_return,
            'max_drawdown': max_drawdown,
            'sharpe_ratio': sharpe,
            'total_trades': len(trades_df),
            'win_rate': win_rate,
            'profit_loss_ratio': profit_loss_ratio,
            'trading_days': days,
            'equity_curve': df,
            'trades': self.trades
        }
    
    def print_result(self, result):
        """打印回测结果"""
        print(f"\n{'='*60}")
        print(f"【回测结果】")
        print(f"{'='*60}")
        
        print(f"\n【收益概况】")
        print(f"  初始资金:    {result['initial_capital']:>12,.0f} 元")
        print(f"  最终权益:    {result['final_equity']:>12,.0f} 元")
        print(f"  总收益率:    {result['total_return']:>12.2f} %")
        print(f"  年化收益率:  {result['annual_return']:>12.2f} %")
        
        print(f"\n【风险指标】")
        print(f"  最大回撤:    {result['max_drawdown']:>12.2f} %")
        print(f"  夏普比率:    {result['sharpe_ratio']:>12.2f}")
        
        print(f"\n【交易统计】")
        print(f"  总交易次数:  {result['total_trades']:>12} 次")
        print(f"  胜率:        {result['win_rate']:>12.2f} %")
        print(f"  盈亏比:      {result['profit_loss_ratio']:>12.2f}")
        print(f"  交易天数:    {result['trading_days']:>12} 天")
        
        print(f"\n{'='*60}")
