#!/usr/bin/env python3
"""
SS v4.0 评分模块 - 三因子模型（优化版）
"""

import pandas as pd
import numpy as np
from data_source_v4 import DataManagerV4


class ScoringV4:
    """评分引擎 v4.0 - 三因子模型（优化版）"""
    
    def __init__(self, dm: DataManagerV4):
        self.dm = dm
        
        # 因子权重 - 调整后
        self.WEIGHT_VALUE = 0.50   # 价值因子（提高：防守优先）
        self.WEIGHT_MOMENTUM = 0.20  # 动量因子（降低：下跌趋势不追涨）
        self.WEIGHT_QUALITY = 0.30   # 质量因子
    
    def calc_all(self, symbol, target_date):
        """
        计算股票综合评分
        返回: dict 包含各因子得分和综合得分
        """
        # 1. 获取数据
        fin = self.dm.get_financial_at_date(symbol, target_date)
        mom = self.dm.get_market_data(symbol, target_date)
        
        if fin is None or mom is None:
            return None
        
        # 2. 计算各因子得分
        value_score = self._calc_value_score(fin)
        momentum_score = self._calc_momentum_score(mom)
        quality_score = self._calc_quality_score(fin)
        
        # 3. 综合得分
        total_score = (
            value_score * self.WEIGHT_VALUE +
            momentum_score * self.WEIGHT_MOMENTUM +
            quality_score * self.WEIGHT_QUALITY
        )
        
        # 4. 生成信号
        signal = self._generate_signal(total_score, value_score, momentum_score)
        
        return {
            'symbol': symbol,
            'target_date': target_date,
            'close': mom['close'],
            # 价值因子
            'value_score': value_score,
            'pe': fin.get('pe_ttm'),
            'pb': fin.get('pb'),
            'dividend_yield': fin.get('dividend_yield'),
            # 动量因子
            'momentum_score': momentum_score,
            'change_20d': mom['change_20d'],
            'change_60d': mom['change_60d'],
            # 质量因子
            'quality_score': quality_score,
            'roe': fin.get('roe'),
            'profit_growth': fin.get('profit_growth'),
            # 综合
            'total_score': total_score,
            'signal': signal,
        }
    
    def _generate_signal(self, total_score, value_score, momentum_score):
        """
        生成交易信号 - 优化版
        增加价值因子门槛：价值得分太低不给买入
        """
        # 价值得分门槛：低于40分不给BUY
        if total_score >= 70 and value_score >= 40:
            return 'BUY'
        elif total_score >= 55:
            return 'WATCH'
        elif total_score < 40:
            return 'SELL'
        else:
            return 'HOLD'
    
    def _calc_value_score(self, fin):
        """
        价值因子得分 (0-100)
        逻辑: PE、PB越低，股息率越高，得分越高
        """
        score = 50  # 基准分
        
        # PE估值
        pe = fin.get('pe_ttm')
        if pe and pe > 0:
            if pe < 10:
                score += 25
            elif pe < 15:
                score += 15
            elif pe < 20:
                score += 10
            elif pe < 30:
                score += 0
            elif pe > 50:
                score -= 20
        
        # PB估值（新增）
        pb = fin.get('pb')
        if pb and pb > 0:
            if pb < 1.5:
                score += 15
            elif pb < 2.5:
                score += 10
            elif pb < 4:
                score += 0
            elif pb > 8:
                score -= 15
        
        # 股息率
        dy = fin.get('dividend_yield')
        if dy:
            if dy > 4:
                score += 15
            elif dy > 2:
                score += 10
            elif dy > 1:
                score += 5
        
        return max(0, min(100, score))
    
    def _calc_momentum_score(self, mom):
        """
        动量因子得分 (0-100) - 优化版
        增加趋势过滤：60日下跌趋势时降低权重
        """
        change_20d = mom.get('change_20d', 0) or 0
        change_60d = mom.get('change_60d', 0) or 0
        
        score = 50  # 基准分
        
        # 趋势过滤：如果60日下跌趋势，动量得分直接砍半
        trend_penalty = 1.0
        if change_60d < -15:
            trend_penalty = 0.5  # 下跌趋势降低权重
        elif change_60d < 0:
            trend_penalty = 0.8
        
        # 20日涨幅 (最佳区间: -10% ~ +15%，不要追涨)
        if -10 <= change_20d <= 15:
            score += 20
        elif -15 <= change_20d <= 25:
            score += 10
        elif change_20d > 25:
            score -= 15  # 涨幅过大不追
        elif change_20d < -15:
            score -= 20  # 跌幅过大不捡
        
        # 60日趋势（带过滤）
        if change_60d > 0:
            score += 15
        elif change_60d > -10:
            score += 0
        else:
            score -= 15
        
        # 应用趋势惩罚
        score = score * trend_penalty
        
        return max(0, min(100, score))
    
    def _calc_quality_score(self, fin):
        """
        质量因子得分 (0-100)
        逻辑: ROE越高、净利润增长越好，得分越高
        """
        score = 50  # 基准分
        
        # ROE
        roe = fin.get('roe')
        if roe:
            if roe > 20:
                score += 25
            elif roe > 15:
                score += 15
            elif roe > 10:
                score += 5
            elif roe < 5:
                score -= 15
        
        # 净利润增长
        pg = fin.get('profit_growth')
        if pg:
            if pg > 20:
                score += 20
            elif pg > 10:
                score += 10
            elif pg > 0:
                score += 5
            elif pg < -20:
                score -= 20
        
        # 毛利率
        gm = fin.get('gross_margin')
        if gm:
            if gm > 40:
                score += 10
            elif gm > 30:
                score += 5
            elif gm < 15:
                score -= 10
        
        return max(0, min(100, score))
    
    def rank_stocks(self, symbols, target_date, top_n=20):
        """
        对股票池评分并排序
        返回 DataFrame
        """
        results = []
        
        for symbol in symbols:
            score = self.calc_all(symbol, target_date)
            if score:
                results.append(score)
        
        # 转 DataFrame
        df = pd.DataFrame(results)
        
        if df.empty:
            return df
        
        # 按综合得分排序
        df = df.sort_values('total_score', ascending=False).head(top_n)
        
        return df


# 测试
if __name__ == "__main__":
    dm = DataManagerV4()
    scorer = ScoringV4(dm)
    
    print("=" * 60)
    print("测试: 茅台评分（优化版）")
    result = scorer.calc_all("600519", "20231231")
    if result:
        print(f"  综合得分: {result['total_score']:.1f}")
        print(f"  价值得分: {result['value_score']:.1f} (PE: {result.get('pe')}, PB: {result.get('pb')})")
        print(f"  动量得分: {result['momentum_score']:.1f} (20日: {result['change_20d']:.2f}%, 60日: {result['change_60d']:.2f}%)")
        print(f"  质量得分: {result['quality_score']:.1f} (ROE: {result['roe']}%)")
        print(f"  信号: {result['signal']}")
    
    print("\n✓ 评分模块测试完成!")
