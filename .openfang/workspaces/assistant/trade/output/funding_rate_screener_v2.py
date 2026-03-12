#!/usr/bin/env python3
"""
资金费率套利筛选器 V2 (优化版)
使用批量接口，大幅提升速度
"""

import requests
import json
from datetime import datetime, timezone, timedelta
import pytz

OKX_API = "https://www.okx.com"
CHINA_TZ = pytz.timezone('Asia/Shanghai')

def get_all_funding_rates():
    """批量获取所有合约资金费率"""
    # 1. 先获取所有USDT合约
    url = f"{OKX_API}/api/v5/public/instruments"
    params = {"instType": "SWAP", "uly": ""}
    
    resp = requests.get(url, params=params, timeout=15)
    data = resp.json()
    
    if data.get("code") != "0":
        print("获取合约列表失败")
        return []
    
    # 筛选USDT合约
    usdt_swaps = [item["instId"] for item in data["data"] if item["instId"].endswith("-USDT-SWAP")]
    
    print(f"📊 共 {len(usdt_swaps)} 个USDT永续合约")
    
    # 2. 批量获取资金费率 (每次最多20个)
    all_funding = []
    batch_size = 20
    
    for i in range(0, min(100, len(usdt_swaps)), batch_size):  # 先查100个
        batch = usdt_swaps[i:i+batch_size]
        inst_ids = ",".join(batch)
        
        url = f"{OKX_API}/api/v5/public/funding-rate-batch"
        params = {"instId": inst_ids}
        
        try:
            resp = requests.get(url, params=params, timeout=10)
            result = resp.json()
            if result.get("code") == "0":
                all_funding.extend(result.get("data", []))
        except:
            pass
        
        if i % 40 == 0:
            print(f"   资金费率进度: {i}/{min(100, len(usdt_swaps))}")
    
    return all_funding

def get_ticker_batch(inst_ids):
    """批量获取行情数据"""
    if not inst_ids:
        return {}
    
    url = f"{OKX_API}/api/v5/market/ticker"
    inst_id_str = ",".join(inst_ids[:100])  # 最多100个
    
    try:
        resp = requests.get(url, params={"instId": inst_id_str}, timeout=10)
        result = resp.json()
        if result.get("code") == "0":
            tickers = {}
            for item in result.get("data", []):
                tickers[item["instId"]] = {
                    "volCcy24h": float(item.get("volCcy24h", 0)),
                    "last": float(item.get("last", 0))
                }
            return tickers
    except:
        pass
    return {}

def screen():
    print(f"\n⏰ 筛选时间: {datetime.now(CHINA_TZ).strftime('%Y-%m-%d %H:%M:%S')}\n")
    
    # 获取资金费率
    print("🔍 正在获取资金费率...")
    funding_data = get_all_funding_rates()
    print(f"   获取到 {len(funding_data)} 条费率数据\n")
    
    # 提取inst_id列表
    inst_ids = [f["instId"] for f in funding_data if f.get("fundingRate")]
    
    # 批量获取成交量
    print("📊 正在获取成交量...")
    tickers = get_ticker_batch(inst_ids)
    print(f"   获取到 {len(tickers)} 条行情数据\n")
    
    # 筛选
    opportunities = []
    for f in funding_data:
        inst_id = f.get("instId", "")
        funding_rate = float(f.get("fundingRate", 0))
        
        # 过滤条件
        if funding_rate <= 0:
            continue
        
        ticker = tickers.get(inst_id, {})
        vol_usd = ticker.get("volCcy24h", 0)
        
        if vol_usd < 5_000_000:  # 成交量至少500万
            continue
        
        symbol = inst_id.replace("-USDT-SWAP", "")
        
        # 年化收益 (每天3次结算)
        annual_return = funding_rate * 3 * 365 * 100
        
        opportunities.append({
            "symbol": symbol,
            "inst_id": inst_id,
            "funding_rate": funding_rate,
            "annual_return": annual_return,
            "volume_usd": vol_usd,
            "price": ticker.get("last", 0)
        })
    
    # 排序
    opportunities.sort(key=lambda x: x["annual_return"], reverse=True)
    
    return opportunities[:10]

def print_results(opps):
    print("\n" + "="*75)
    print("🎯 资金费率套利机会 - TOP 10")
    print("="*75)
    print(f"{'排名':<4} {'币种':<8} {'费率/8h':<10} {'年化收益':<10} {'24h成交量':<12} {'价格'}")
    print("-"*75)
    
    for i, o in enumerate(opps, 1):
        print(f"{i:<4} {o['symbol']:<8} "
              f"{o['funding_rate']*100:>6.4f}%  "
              f"{o['annual_return']:>7.2f}%   "
              f"${o['volume_usd']/1e6:>6.1f}M    "
              f"${o['price']:>10.2f}")
    
    print("-"*75)
    
    if opps:
        best = opps[0]
        print(f"\n🏆 最佳推荐: {best['symbol']}")
        print(f"   📈 年化收益: {best['annual_return']:.2f}%")
        
        # 不同本金收益
        for capital in [1000, 5000, 10000, 50000]:
            daily = capital * best['funding_rate'] * 3
            monthly = daily * 30
            print(f"   💰 本金${capital:,}/天: ${daily:.2f} | 月: ${monthly:.2f}")
        
        # TOP3 组合
        if len(opps) >= 3:
            avg = sum(o['annual_return'] for o in opps[:3]) / 3
            print(f"\n📊 TOP3 组合平均年化: {avg:.2f}%")

if __name__ == "__main__":
    opps = screen()
    print_results(opps)
