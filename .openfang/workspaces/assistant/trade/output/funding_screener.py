#!/usr/bin/env python3
"""
资金费率筛选器 - 简化版
"""

import requests
import json
from datetime import datetime, timezone
import pytz

OKX = "https://www.okx.com"
CHINA_TZ = pytz.timezone('Asia/Shanghai')

def timestamp_to_time(ts):
    """时间戳转时间字符串"""
    try:
        dt = datetime.fromtimestamp(int(ts)/1000, tz=timezone.utc)
        return dt.astimezone(CHINA_TZ).strftime("%H:%M")
    except:
        return "N/A"

def main():
    print(f"\n⏰ {datetime.now(CHINA_TZ).strftime('%Y-%m-%d %H:%M:%S')}\n")
    
    # 1. 获取USDT永续合约列表
    print("🔍 获取合约列表...")
    r = requests.get(f"{OKX}/api/v5/public/instruments", 
                    params={"instType": "SWAP", "uly": ""}, timeout=15)
    data = r.json()
    
    if data["code"] != "0":
        print("错误:", data["msg"])
        return
    
    # 筛选主流币种
    usdt_swaps = [d["instId"] for d in data["data"] if d["instId"].endswith("-USDT-SWAP")]
    print(f"   共 {len(usdt_swaps)} 个USDT永续合约\n")
    
    # 常用币种列表（优先查这些）
    symbols = ["BTC", "ETH", "SOL", "XRP", "DOGE", "ADA", "AVAX", "LINK", 
               "MATIC", "DOT", "UNI", "ATOM", "LTC", "FIL", "APT", "ARB", 
               "OP", "NEAR", "SHIB", "PEPE", "BNB", "TRX", "XLM", "ALGO"]
    
    results = []
    
    print("📊 获取资金费率...\n")
    
    for sym in symbols:
        inst_id = f"{sym}-USDT-SWAP"
        if inst_id not in usdt_swaps:
            continue
        
        # 获取资金费率
        r = requests.get(f"{OKX}/api/v5/public/funding-rate",
                        params={"instId": inst_id}, timeout=5)
        fd = r.json()
        
        # 获取行情
        r = requests.get(f"{OKX}/api/v5/market/ticker",
                        params={"instId": inst_id}, timeout=5)
        tk = r.json()
        
        if fd["code"] == "0" and fd["data"] and tk["code"] == "0" and tk["data"]:
            fr = fd["data"][0]
            t = tk["data"][0]
            
            funding_rate = float(fr["fundingRate"])
            vol_usd = float(t["volCcy24h"])
            price = float(t["last"])
            
            # 年化 (每天3次)
            annual = funding_rate * 3 * 365 * 100
            
            # 下次结算时间
            next_time = timestamp_to_time(fr.get("nextFundingTime", ""))
            
            results.append({
                "symbol": sym,
                "rate": funding_rate,
                "annual": annual,
                "volume": vol_usd,
                "price": price,
                "next": next_time
            })
            
            print(f"   {sym}: 费率 {funding_rate*100:.4f}% | 年化 {annual:.1f}% | 成交量 ${vol_usd/1e6:.1f}M")
    
    # 排序
    results.sort(key=lambda x: x["annual"], reverse=True)
    
    # 打印结果
    print("\n" + "="*75)
    print("🎯 资金费率套利机会 - TOP 10")
    print("="*75)
    print(f"{'排名':<4} {'币种':<6} {'费率/8h':<10} {'年化收益':<10} {'24h成交':<10} {'下次结算'}")
    print("-"*75)
    
    for i, r in enumerate(results[:10], 1):
        print(f"{i:<4} {r['symbol']:<6} {r['rate']*100:>7.4f}%   {r['annual']:>7.1f}%    "
              f"${r['volume']/1e6:>5.1f}M    {r['next']}")
    
    print("-"*75)
    
    if results:
        best = results[0]
        print(f"\n🏆 最佳推荐: {best['symbol']}")
        print(f"   📈 年化收益: {best['annual']:.2f}%")
        
        for cap in [1000, 5000, 10000, 50000]:
            daily = cap * best['rate'] * 3
            print(f"   💰 ${cap:>5,} 本金 → 日收益: ${daily:.2f} | 月: ${daily*30:.2f}")

if __name__ == "__main__":
    main()
