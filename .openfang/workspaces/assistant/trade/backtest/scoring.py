#!/usr/bin/env python3
"""
SS v3.0 量化投资框架 - 核心评分模块
"""

import pandas as pd
import numpy as np
from datetime import datetime
from typing import Dict
import os

from backtest.data_source import DataManager


class SSScoring:
    """SS v3.0 评分计算器"""
    
    def __init__(self, data_manager: DataManager = None):
        self.dm = data_manager or DataManager()
    
    def calc_basic_quality(self, symbol: str, financial: Dict = None) -> float:
        if financial is None:
            financial = self.dm.get_financial(symbol)
        
        # 处理 DataFrame 情况
        if financial is not None and hasattr(financial, 'iloc'):
            if len(financial) > 0:
                financial = financial.iloc[0].to_dict()
        
        # 安全检查
        if financial is None:
            return 50.0
        
        roe = financial.get('roe', 0) or 0
        if roe >= 20:
            roe_score = 100
        elif roe >= 10:
            roe_score = 70
        elif roe >= 5:
            roe_score = 50
        else:
            roe_score = 30
        
        rev_growth = financial.get('revenue_growth', 0) or 0
        if rev_growth >= 30:
            rev_score = 100
        elif rev_growth >= 15:
            rev_score = 70
        elif rev_growth >= 0:
            rev_score = 50
        else:
            rev_score = 20
        
        gross_margin = financial.get('gross_margin', 0) or 0
        if gross_margin >= 40:
            margin_score = 100
        elif gross_margin >= 20:
            margin_score = 70
        elif gross_margin >= 10:
            margin_score = 50
        else:
            margin_score = 30
        
        cash_flow = financial.get('cash_flow_ratio', 0) or 0
        if cash_flow >= 10:
            cash_score = 100
        elif cash_flow >= 0:
            cash_score = 70
        else:
            cash_score = 30
        
        market_cap = financial.get('market_cap', 0) or 0
        if market_cap >= 1000:
            position_score = 100
        elif market_cap >= 500:
            position_score = 70
        elif market_cap >= 100:
            position_score = 50
        else:
            position_score = 30
        
        score = (roe_score * 0.2 + rev_score * 0.2 + margin_score * 0.2 + 
                 cash_score * 0.2 + position_score * 0.2)
        
        return round(score, 1)
    
    def calc_industry_position(self, symbol: str) -> float:
        info = self.dm.get_stock_info(symbol)
        
        if info is None:
            return 50.0
        
        industry = info.get('industry', info.get('industry_name', 'unknown'))
        
        industry_benchmark = {
            '半导体': 80,
            '新能源': 75,
            '医药': 65,
            '消费': 60,
            '金融': 50,
        }
        景气度 = industry_benchmark.get(industry, 50)
        
        # 尝试从info中获取市值
        market_cap = 0
        if 'total_share' in info:
            try:
                shares = float(info.get('total_share', 0))
                price = float(info.get('close', 0))
                market_cap = shares * price / 100000000  # 转换为亿
            except:
                pass
        
        if market_cap >= 1000:
            competition = 80
        elif market_cap >= 500:
            competition = 65
        elif market_cap >= 100:
            competition = 50
        else:
            competition = 35
        
        policy = 70
        
        score = 景气度 * 0.4 + competition * 0.3 + policy * 0.3
        return round(score, 1)
    
    def calc_valuation(self, symbol: str, financial: Dict = None) -> float:
        if financial is None:
            financial = self.dm.get_financial(symbol)
        
        # 处理 DataFrame 情况
        if financial is not None and hasattr(financial, 'iloc'):
            if len(financial) > 0:
                financial = financial.iloc[0].to_dict()
        
        if financial is None:
            return 50.0
        
        pe = financial.get('pe_ttm', 50) or 50
        pb = financial.get('pb', 5) or 5
        
        if pe <= 20:
            pe_score = 100
        elif pe <= 30:
            pe_score = 80
        elif pe <= 50:
            pe_score = 60
        elif pe <= 70:
            pe_score = 40
        else:
            pe_score = 20
        
        if pb <= 2:
            pb_score = 100
        elif pb <= 5:
            pb_score = 70
        elif pb <= 10:
            pb_score = 50
        else:
            pb_score = 30
        
        relative_score = 50
        
        score = pe_score * 0.4 + pb_score * 0.3 + relative_score * 0.3
        return round(score, 1)
    
    def calc_money_structure(self, symbol: str, days: int = 20) -> float:
        flow = self.dm.get_money_flow(symbol, days)
        info = self.dm.get_stock_info(symbol)
        
        if flow is None or len(flow) == 0:
            return 50.0
        
        try:
            inflow_5d = float(flow['main_net_inflow_5d'].iloc[-1])
        except:
            inflow_5d = 0
        
        if inflow_5d >= 5:
            inflow_score = 100
        elif inflow_5d >= 1:
            inflow_score = 70
        elif inflow_5d >= 0:
            inflow_score = 50
        else:
            inflow_score = 20
        
        holder_num = 50000
        if info:
            holder_num = info.get('holder_num', 50000) or 50000
        
        if holder_num < 30000:
            holder_score = 100
        elif holder_num < 50000:
            holder_score = 70
        else:
            holder_score = 50
        
        institution_score = 60
        
        score = inflow_score * 0.4 + holder_score * 0.3 + institution_score * 0.3
        return round(score, 1)
    
    def calc_morphology(self, symbol: str, daily: pd.DataFrame = None) -> float:
        if daily is None or len(daily) < 60:
            return 50.0
        
        close = daily['close']
        ma5 = close.rolling(5).mean()
        ma20 = close.rolling(20).mean()
        ma60 = close.rolling(60).mean()
        
        current_price = close.iloc[-1]
        ma20_val = ma20.iloc[-1]
        ma60_val = ma60.iloc[-1]
        
        if current_price > ma20_val and current_price > ma60_val:
            trend_score = 80
        elif current_price > ma20_val or current_price > ma60_val:
            trend_score = 60
        else:
            trend_score = 30
        
        if ma5.iloc[-1] > ma20.iloc[-1] > ma60.iloc[-1]:
            ma_score = 100
        elif ma20.iloc[-1] > ma60.iloc[-1]:
            ma_score = 60
        else:
            ma_score = 30
        
        high_20 = close.tail(20).max()
        low_20 = close.tail(20).min()
        
        if current_price >= high_20 * 0.95:
            pos_score = 100
        elif current_price <= low_20 * 1.05:
            pos_score = 50
        else:
            pos_score = 70
        
        score = trend_score * 0.3 + ma_score * 0.3 + pos_score * 0.4
        return round(score, 1)
    
    def calc_price_volume(self, symbol: str, daily: pd.DataFrame = None) -> float:
        if daily is None or len(daily) < 20:
            return 50.0
        
        close = daily['close']
        vol = daily['volume']
        
        price_change = (close.iloc[-1] / close.iloc[-5] - 1) * 100
        vol_change = (vol.iloc[-5:].mean() / vol.iloc[-20:-5].mean() - 1) * 100
        
        if price_change > 0 and vol_change > 0:
            vol_score = 100
        elif price_change < 0 and vol_change < 0:
            vol_score = 100
        elif price_change > 0 and vol_change < 0:
            vol_score = 40
        elif price_change < 0 and vol_change > 0:
            vol_score = 60
        else:
            vol_score = 50
        
        high_20 = close.tail(20).max()
        vol_20_avg = vol.tail(20).mean()
        
        if close.iloc[-1] >= high_20 * 0.98 and vol.iloc[-1] < vol_20_avg * 0.8:
            divergence_score = 30
        else:
            divergence_score = 70
        
        score = vol_score * 0.5 + divergence_score * 0.5
        return round(score, 1)
    
    def calc_volatility_risk(self, symbol: str, daily: pd.DataFrame = None) -> float:
        if daily is None or len(daily) < 20:
            return 50.0
        
        close = daily['close'].tail(20)
        returns = close.pct_change().dropna()
        vol = returns.std() * np.sqrt(252) * 100
        
        if vol < 20:
            vol_score = 100
        elif vol < 40:
            vol_score = 70
        elif vol < 60:
            vol_score = 50
        else:
            vol_score = 30
        
        cummax = close.cummax()
        drawdown = (close - cummax) / cummax * 100
        max_dd = drawdown.min()
        
        if max_dd > -5:
            dd_score = 100
        elif max_dd > -10:
            dd_score = 70
        elif max_dd > -20:
            dd_score = 50
        else:
            dd_score = 30
        
        score = vol_score * 0.5 + dd_score * 0.5
        return round(score, 1)
    
    def calc_event_risk(self, symbol: str) -> float:
        performance = 60
        restricted = 70
        reduction = 70
        
        score = performance * 0.4 + restricted * 0.3 + reduction * 0.3
        return round(score, 1)
    
    def calc_liquidity_risk(self, symbol: str, daily: pd.DataFrame = None) -> float:
        if daily is None or len(daily) < 20:
            return 50.0
        
        avg_amount = daily['amount'].tail(20).mean() / 100000000
        
        if avg_amount >= 10:
            amount_score = 100
        elif avg_amount >= 5:
            amount_score = 70
        elif avg_amount >= 1:
            amount_score = 50
        else:
            amount_score = 30
        
        depth_score = 70
        
        score = amount_score * 0.5 + depth_score * 0.5
        return round(score, 1)
    
    def calc_all(self, symbol: str, date: str = None) -> Dict:
        if date:
            end_date = datetime.strptime(date, "%Y%m%d").strftime("%Y%m%d")
            start_date = (datetime.strptime(date, "%Y%m%d") - 
                         pd.Timedelta(days=90)).strftime("%Y%m%d")
        else:
            end_date = datetime.now().strftime("%Y%m%d")
            start_date = (datetime.now() - pd.Timedelta(days=90)).strftime("%Y%m%d")
        
        financial = self.dm.get_financial(symbol)
        
        # 处理 financial 可能返回 DataFrame 的情况
        if financial is not None and hasattr(financial, 'iloc'):
            if len(financial) > 0:
                financial = financial.iloc[0].to_dict()
        
        daily = self.dm.get_daily(symbol, start_date, end_date)
        
        basic_quality = self.calc_basic_quality(symbol, financial)
        industry_position = self.calc_industry_position(symbol)
        valuation = self.calc_valuation(symbol, financial)
        value_layer = (basic_quality + industry_position + valuation) / 3
        
        money_structure = self.calc_money_structure(symbol)
        morphology = self.calc_morphology(symbol, daily)
        price_volume = self.calc_price_volume(symbol, daily)
        trade_layer = (money_structure + morphology + price_volume) / 3
        
        volatility_risk = self.calc_volatility_risk(symbol, daily)
        event_risk = self.calc_event_risk(symbol)
        liquidity_risk = self.calc_liquidity_risk(symbol, daily)
        risk_layer = (volatility_risk + event_risk + liquidity_risk) / 3
        
        total_score = (value_layer + trade_layer + risk_layer) / 3
        
        if value_layer >= 65 and trade_layer >= 65 and risk_layer >= 65:
            rank = 'A'
        elif value_layer >= 55 and trade_layer >= 55 and risk_layer >= 55:
            rank = 'B'
        elif value_layer >= 50 or trade_layer >= 50 or risk_layer >= 50:
            rank = 'C'
        else:
            rank = 'D'
        
        if trade_layer >= 60 and rank in ['A', 'B']:
            signal = 'BUY'
        elif rank == 'C':
            signal = 'WATCH'
        elif rank == 'D':
            signal = 'SELL'
        else:
            signal = 'HOLD'
        
        variance = np.std([value_layer, trade_layer, risk_layer])
        confidence = max(0.3, min(0.9, 1 - variance / 50))
        
        return {
            'symbol': symbol,
            'date': date or end_date,
            'basic_quality': basic_quality,
            'industry_position': industry_position,
            'valuation': valuation,
            'money_structure': money_structure,
            'morphology': morphology,
            'price_volume': price_volume,
            'volatility_risk': volatility_risk,
            'event_risk': event_risk,
            'liquidity_risk': liquidity_risk,
            'value_layer': round(value_layer, 1),
            'trade_layer': round(trade_layer, 1),
            'risk_layer': round(risk_layer, 1),
            'total_score': round(total_score, 1),
            'rank': rank,
            'signal': signal,
            'confidence': round(confidence, 2)
        }
    
    def print_report(self, result: Dict):
        print("\n" + "=" * 60)
        print(f"           SS v3.0 评分报告 - {result['symbol']}")
        print("=" * 60)
        print(f"分析日期: {result['date']}")
        
        print("\n【价值层】")
        print(f"  基本面质量:    {result['basic_quality']}")
        print(f"  行业位势:      {result['industry_position']}")
        print(f"  估值赔率:      {result['valuation']}")
        print(f"  ─────────────  {result['value_layer']}")
        
        print("\n【交易层】")
        print(f"  资金结构:      {result['money_structure']}")
        print(f"  裸K形态:       {result['morphology']}")
        print(f"  量价配合:      {result['price_volume']}")
        print(f"  ─────────────  {result['trade_layer']}")
        
        print("\n【风险层】")
        print(f"  波动风险:      {result['volatility_risk']}")
        print(f"  事件风险:      {result['event_risk']}")
        print(f"  流动性风险:    {result['liquidity_risk']}")
        print(f"  ─────────────  {result['risk_layer']}")
        
        print("\n【综合结果】")
        print(f"  总分:          {result['total_score']}")
        print(f"  档位:          {result['rank']} 级")
        print(f"  信号:          {result['signal']}")
        print(f"  置信度:        {result['confidence']}")
        print("=" * 60)


if __name__ == "__main__":
    print("SS v3.0 评分模块测试")
    print("=" * 50)
    
    dm = DataManager()
    scorer = SSScoring(dm)
    
    result = scorer.calc_all("688008")
    scorer.print_report(result)
