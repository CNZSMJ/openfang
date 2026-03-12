# A股复盘工具

基于 daily_stock_analysis 项目思想设计的轻量级复盘工具。

## 数据获取

使用 akshare 获取：
- `stock_zt_pool_em()` - 涨停股
- `stock_zh_a_spot_em()` - 实时行情（跌停筛选）
- `stock_zh_index_daily()` - 指数行情

## 复盘模板

```
# 2026年X月X日 A股复盘

## 一、大盘概况
| 指数 | 收盘价 | 涨跌幅 |
|------|--------|--------|

涨停: X只 | 跌停: X只 | 封板率: XX%

## 二、涨停板块
### 1. 板块名称
| 代码 | 名称 | 涨幅 | 备注 |
|------|------|------|------|

## 三、跌停股
| 代码 | 名称 | 跌幅 | 备注 |
|------|------|------|------|

## 四、总结
### 赚钱效应
### 风险提示
```

## 使用方法

```bash
# 激活环境
source ~/venv_akshare/bin/activate

# 运行复盘
python daily_review.py
```

## 目录结构

```
daily_stock_analysis/
├── daily_review.py      # 主程序
├── templates/           # 复盘模板
│   └── review_template.md
├── data/               # 数据缓存
└── output/             # 生成的复盘报告
    └── 20260309.md
```
