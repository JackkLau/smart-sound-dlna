import argparse
import threading
import webbrowser
from pathlib import Path

from .server import create_server


def main():
    parser = argparse.ArgumentParser(description="小米音响 DLNA 本地音乐投送工具（Python 3.9+，无需额外依赖）")
    parser.add_argument("--port", type=int, default=18200, help="HTTP 端口，默认 18200")
    parser.add_argument("--advertise-host", help="指定音响可访问的本机局域网 IPv4 地址")
    parser.add_argument("--interface", help="用于搜索音响的本机网卡 IPv4 地址")
    parser.add_argument("--data-dir", type=Path, default=Path.cwd() / ".data", help="音乐库和导入文件保存位置")
    parser.add_argument("--music-dir", help="启动时扫描的本机音乐文件夹（不复制文件）")
    parser.add_argument("--no-browser", action="store_true", help="不自动打开浏览器")
    args = parser.parse_args()
    if not 1 <= args.port <= 65535:
        parser.error("端口必须在 1 到 65535 之间。")
    try:
        server = create_server(port=args.port, data_dir=args.data_dir,
                               advertise_host=args.advertise_host, interface=args.interface)
        if args.music_dir:
            result = server.app.library.scan(args.music_dir)
            print("音乐库：%s 首（本次新增 %s 首）" % (result["total"], result["added"]))
    except (OSError, ValueError) as exc:
        parser.exit(1, "启动失败：%s\n" % exc)
    url = "http://127.0.0.1:%s" % server.app.port
    print("\n小米音乐投送已启动：%s\n" % url)
    print("请让电脑与音响连接同一局域网，并在小爱音箱 App 中开启 DLNA。")
    print("控制页面仅限本机访问；保持此程序运行，按 Ctrl+C 退出。\n")
    if not args.no_browser:
        timer = threading.Timer(0.6, lambda: webbrowser.open(url))
        timer.daemon = True
        timer.start()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\n已停止音乐服务。")
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
