# 初始电影片库的数据来源

`movie-seed.json` 保存首次建库所需的豆瓣电影条目编号、片名、原名（可空）、上映年份、评分与观察日期。评分乘以十后用整数保存，80 代表 8.0 分。年份以作品首映年份为准。历史榜单或工作簿保留对应快照日期，补录和重新打包不会把日期改成当天。

此版共收录 1,155 个不同的豆瓣电影编号。其中 760 部采用 2026-11-27 的公开页面观测，395 部采用 2026-04-20 的归档观测；两条已确认的磁力版本包含在后者中。

2026 年 11 月核对的范围包括电影 Top 250 的十页、新片榜，以及下列公开电影片单。片单用于发现电影，数值取条目旁的豆瓣评分。片单名称中的其他平台评分不参与筛选。

- [电影 Top 250](https://movie.douban.com/top250) · [新片榜](https://movie.douban.com/chart)
- [华语续榜](https://www.douban.com/doulist/160718465/) · [华语电影](https://www.douban.com/doulist/123435310/) · [国产电影](https://www.douban.com/doulist/157864682/)
- [日本电影与剧场版](https://www.douban.com/doulist/148800689/) · [日本电影](https://www.douban.com/doulist/148927886/) · [韩国电影](https://www.douban.com/doulist/148818807/)
- [其他地区选片](https://www.douban.com/doulist/162585/) · [近期电影片单](https://www.douban.com/doulist/160806268/) · [近期片单二](https://www.douban.com/doulist/160641557/) · [近期片单三](https://www.douban.com/doulist/164826864/)

公开分类榜的历史采集使用固定版本的工作簿。工作簿的 release_date 可能是重映日期，核对作品年份后才可加入。从工作簿补充的 393 部另对照详情页的 ID、Movie 类型、片头年份和评分，其中 24 部纠正了重映日期造成的年份偏差。历史记录和同一电影较新的观测冲突时，使用较新的观测。[2026 年 4 月榜单归档](https://github.com/aaamqrx/doban-data/tree/89aa62a129ec61c720392ba35774437d70e1d025/data/raw) · [用于核实作品年份的详情页归档](https://github.com/aaamqrx/doban-data/tree/4592522246ef7faa3a97412fab21da764ffe77e8/html_cache)

`open-film-resources.json` 另存已核对的发行版本。每条数据包括公开发布的磁力标识、发布页、许可说明和可复核的评分出处。《蓝调之歌》和《互联网之子》的评分分别见 2026 年 4 月 20 日归档的奇幻榜第 1073 行、纪录片榜第 77 行。资源数由实际存在的 BTIH 与影片关联计算，在线节点数由启动下载后的引擎观测。
