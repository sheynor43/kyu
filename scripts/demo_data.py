"""Writes a demo data folder used for the README screenshots.

Usage: python3 scripts/demo_data.py <dir>
Then run the app with KYU_DATA_DIR=<dir> KYU_OFFLINE=1.
"""
import json, os, sys, datetime as dt

root = sys.argv[1]
os.makedirs(os.path.join(root, "days"), exist_ok=True)
now = dt.datetime.now().astimezone()
today = (now - dt.timedelta(hours=6)).date()

def item(i, mins_ago, queue, origin, user, kind, url, msg="", status="pending"):
    t = now - dt.timedelta(minutes=mins_ago)
    return {"id": 1_000_000 + i, "received": t.isoformat(), "queue": queue, "origin": origin,
            "user": user, "kind": kind, "url": url, "message": msg, "status": status,
            "done_at": None if status == "pending" else now.isoformat()}

da = lambda a: {"type": "donation_alerts", "amount": a, "currency": "RUB"}
dp = lambda a: {"type": "donate_pay", "amount": a, "currency": "RUB"}
rw = {"type": "reward", "reward_id": "7f3c1d2e-91aa-4b6e-8f21-0c5d9e4a1b77"}
chat = {"type": "chat"}

items = [
    item(1, 94, "paid", da(500), "lumi_taps", "score", "https://osu.ppy.sh/scores/4721839905", "первый 7*, глянь пж", "watched"),
    item(2, 41, "paid", da(300), "kotowari", "score", "https://osu.ppy.sh/scores/4723310457", "фц на стриме вчера"),
    item(3, 28, "paid", dp(150), "nyxwave", "profile", "https://osu.ppy.sh/users/18342977", "оцени профиль"),
    item(4, 12, "paid", {"type": "bits", "bits": 500}, "Sh1ro_osu", "score", "https://osu.ppy.sh/scores/osu/4724018832", "dt 98%"),
    item(5, 63, "points", rw, "mapleleaf", "score", "https://osu.ppy.sh/scores/4722450016", "", "watched"),
    item(6, 33, "points", rw, "pixelcat", "profile", "https://osu.ppy.sh/users/12808443"),
    item(7, 19, "points", rw, "tsukimi", "score", "https://osu.ppy.sh/scores/4723881290", "мой топ скор"),
    item(8, 7, "points", rw, "deltaburst", "score", "https://osu.ppy.sh/scores/4724105573"),
    item(9, 52, "free", chat, "gaz1k", "profile", "https://osu.ppy.sh/users/21590034"),
    item(10, 37, "free", chat, "riverstone", "score", "https://osu.ppy.sh/scores/4723197761", "hd hr", "skipped"),
    item(11, 24, "free", chat, "ame_no", "score", "https://osu.ppy.sh/scores/4723602118"),
    item(12, 9, "free", chat, "vortexx", "profile", "https://osu.ppy.sh/users/15402226", "только начал играть"),
    item(13, 3, "free", chat, "hikari_9", "score", "https://osu.ppy.sh/scores/4724199034"),
]
json.dump({"items": items}, open(os.path.join(root, "days", f"{today}.json"), "w"), ensure_ascii=False, indent=2)

# a few older days so the sidebar shows history
for back, n, done in [(1, 9, 9), (2, 14, 11), (4, 6, 6), (5, 11, 11), (8, 7, 5), (12, 10, 10), (33, 4, 4)]:
    d = today - dt.timedelta(days=back)
    its = []
    for k in range(n):
        st = "watched" if k < done else "pending"
        its.append(item(100 * back + k, 60 * 24 * back + k * 7, ["paid", "points", "free"][k % 3],
                        [da(200), rw, chat][k % 3], f"viewer{k}", "score",
                        f"https://osu.ppy.sh/scores/47{back:02d}{k:06d}", "", st))
    json.dump({"items": its}, open(os.path.join(root, "days", f"{d}.json"), "w"), indent=2)

settings = {"lang": "ru", "twitch_channel": "lumi_osu", "da_token": "x", "dp_token": "",
            "known_rewards": [{"id": "7f3c1d2e-91aa-4b6e-8f21-0c5d9e4a1b77", "sample": "https://osu.ppy.sh/scores/4723881290 мой топ скор"}],
            "reward_ids": ["7f3c1d2e-91aa-4b6e-8f21-0c5d9e4a1b77"]}
if len(sys.argv) > 2:
    settings["lang"] = sys.argv[2]
json.dump(settings, open(os.path.join(root, "settings.json"), "w"), indent=2)
