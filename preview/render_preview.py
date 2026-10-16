"""绘制供审阅的静态页面图，不包含应用窗口、交互或正式界面实现。"""
from pathlib import Path
import json
from PIL import Image, ImageDraw, ImageFont, PngImagePlugin


ROOT = Path(__file__).resolve().parents[1]
OUT = Path(__file__).resolve().parent
evidence = json.loads((ROOT / "verification/codex-multi-continuing.json").read_text(encoding="utf-8"))
processes = json.loads((ROOT / "verification/codex-multi-continuing-processes.json").read_text(encoding="utf-8-sig"))
assert evidence["passed"] and processes["sameProcesses"], "三会话验证通过后才制作预览"

S = 2
W, H = 1440, 900
C = {
    "ink": "#202936", "sub": "#566273", "muted": "#647184",
    "line": "#E0E5EC", "canvas": "#F5F7FA", "nav": "#F0F3F7",
    "white": "#FFFFFF", "blue": "#265FC5", "bluebg": "#E7EEFC",
    "green": "#24694E", "greenbg": "#EAF5EF", "amber": "#845016",
    "amberbg": "#FFF3DC", "code": "#F1F4F8", "soft": "#FAFBFD",
}
fonts = {}


def font(size=15, bold=False, mono=False):
    key = (size, bold, mono)
    if key not in fonts:
        path = "C:/Windows/Fonts/consola.ttf" if mono else (
            "C:/Windows/Fonts/msyhbd.ttc" if bold else "C:/Windows/Fonts/msyh.ttc")
        fonts[key] = ImageFont.truetype(path, round(size * S))
    return fonts[key]


class Page:
    def __init__(self):
        self.image = Image.new("RGB", (W * S, H * S), C["canvas"])
        self.d = ImageDraw.Draw(self.image)
        self.bounds = []

    def rect(self, box, fill, outline=None, radius=0):
        box = tuple(round(v * S) for v in box)
        if radius:
            self.d.rounded_rectangle(box, radius=round(radius * S), fill=fill, outline=outline, width=S)
        else:
            self.d.rectangle(box, fill=fill, outline=outline, width=S)

    def line(self, points, fill=None, width=1):
        self.d.line([(round(x * S), round(y * S)) for x, y in points],
                    fill=fill or C["line"], width=round(width * S), joint="curve")

    def circle(self, x, y, r, fill, outline=None, width=1):
        self.d.ellipse(((x-r)*S, (y-r)*S, (x+r)*S, (y+r)*S),
                       fill=fill, outline=outline, width=round(width*S))

    def text(self, x, y, value, size=15, color=None, bold=False, mono=False, maxw=None):
        f = font(size, bold, mono)
        value = str(value)
        if maxw is not None and self.d.textlength(value, font=f) > maxw*S:
            while value and self.d.textlength(value + "…", font=f) > maxw*S:
                value = value[:-1]
            value += "…"
        self.d.text((round(x*S), round(y*S)), value, font=f, fill=color or C["ink"], anchor="lt")
        b = self.d.textbbox((round(x*S), round(y*S)), value, font=f, anchor="lt")
        self.bounds.append(tuple(v/S for v in b))
        return self.d.textlength(value, font=f)/S

    def para(self, x, y, value, width, size=15, color=None, lineh=26, bold=False):
        lines = []
        f = font(size, bold)
        for raw in value.split("\n"):
            row = ""
            for char in raw:
                if row and self.d.textlength(row + char, font=f) > width*S:
                    if char in "，。、；：！？）》】”’" and len(row) > 1:
                        lines.append(row[:-1])
                        row = row[-1] + char
                    else:
                        lines.append(row)
                        row = char
                else:
                    row += char
            lines.append(row)
        for i, row in enumerate(lines):
            self.text(x, y+i*lineh, row, size, color, bold)
        return y + len(lines)*lineh

    def icon(self, name, x, y, color=None, size=18):
        color = color or C["sub"]
        k = size/18
        def line(points, width=1.6):
            self.line([(x+u*k, y+v*k) for u, v in points], color, width)
        if name == "search":
            self.circle(x+7*k, y+7*k, 4.7*k, None, color, 1.5)
            line([(11,11),(16,16)])
        elif name == "arrow":
            line([(2,9),(15,9)]); line([(10,4),(15,9),(10,14)])
        elif name == "back":
            line([(3,9),(16,9)]); line([(8,4),(3,9),(8,14)])
        elif name == "chevron":
            line([(6,4),(11,9),(6,14)])
        elif name == "check":
            line([(3,9),(7,13),(15,4)])
        elif name == "clock":
            self.circle(x+9*k,y+9*k,7*k,None,color,1.4)
            line([(9,4),(9,9),(12,11)])
        elif name == "link":
            line([(2,10),(6,6),(9,6),(12,9)])
            line([(6,9),(9,12),(12,12),(16,8)])
            line([(6,12),(12,6)])
        elif name == "refresh":
            self.d.arc((x*S,y*S,(x+17*k)*S,(y+17*k)*S),40,310,fill=color,width=3)
            line([(13,1),(17,3),(16,7)])
        elif name == "info":
            self.circle(x+9*k,y+9*k,7*k,None,color,1.4)
            self.circle(x+9*k,y+5*k,0.8*k,color)
            line([(9,8),(9,13)])
        elif name == "warning":
            line([(9,1),(17,16),(1,16),(9,1)])
            line([(9,6),(9,10)]); self.circle(x+9*k,y+13*k,.8*k,color)
        elif name == "copy":
            self.rect((x+6*k,y+6*k,x+15*k,y+16*k),None,color,2)
            line([(4,13),(2,13),(2,2),(12,2),(12,4)])
        elif name == "stream":
            for yy, xx in [(4,13),(9,16),(14,11)]:
                self.circle(x+2*k,y+yy*k,1.1*k,color)
                line([(6,yy),(xx,yy)])
        elif name == "grid":
            for xx in (2,10):
                for yy in (2,10):
                    self.rect((x+xx*k,y+yy*k,x+(xx+5)*k,y+(yy+5)*k),None,color,1)

    def badge(self, x, y, value, kind="blue"):
        fg, bg = {"blue":(C["blue"],C["bluebg"]), "green":(C["green"],C["greenbg"]),
                  "amber":(C["amber"],C["amberbg"]), "gray":(C["sub"],"#EDF0F4")}[kind]
        width = self.d.textlength(value, font=font(12))/S + 20
        self.rect((x,y,x+width,y+25),bg,radius=5)
        self.text(x+10,y+6,value,12,fg)
        return width

    def button(self, x, y, value, width, primary=False, icon=None):
        self.rect((x,y,x+width,y+34),C["blue"] if primary else C["white"],
                  None if primary else "#CDD5DF",5)
        color = C["white"] if primary else C["ink"]
        offset = 12
        if icon:
            self.icon(icon,x+11,y+9,color,16); offset = 34
        self.text(x+offset,y+9,value,13,color)

    def shell(self, detail=False, offline=False):
        self.rect((0,0,W,32),"#F7F8FA")
        self.rect((13,10,24,21),None,C["blue"],2)
        self.line([(16,16),(21,16)],C["blue"],1.6)
        self.text(33,9,"Agent 管理器",12)
        self.text(1040,9,"静态预览 · 演示数据",12,C["sub"])
        self.line([(1302,16),(1314,16)],C["sub"])
        self.rect((1354,11,1364,21),None,C["sub"])
        self.line([(1401,11),(1411,21)],C["sub"])
        self.line([(1411,11),(1401,21)],C["sub"])
        self.rect((0,32,W,92),C["white"])
        self.icon("stream",20,53,C["blue"],22)
        self.text(53,51,"会话工作台",20,bold=True)
        self.line([(190,49),(190,75)])
        self.text(208,57,"本机 / Codex Desktop",14,C["sub"])
        self.badge(1280,51,"只读监控","gray")
        self.line([(0,92),(W,92)])
        self.rect((0,93,218,872),C["nav"])
        self.line([(218,93),(218,872)])
        self.rect((14,110,204,146),C["white"],"#D5DCE6",5)
        self.icon("search",25,120,size=16)
        self.text(50,120,"搜索会话",13,C["sub"])
        if not detail:
            self.rect((14,157,204,186),C["bluebg"],radius=5)
        self.icon("back" if detail else "grid",24,164,C["blue"],16)
        self.text(49,168,"返回会话总览" if detail else "会话总览",13,C["blue"],True)
        self.text(188,168,"3",13,C["sub"])
        entries = [("检查会话同步","检查增量版本","demo-core"),
                   ("修复日志字段","等待测试结果","demo-api"),
                   ("梳理模块依赖","本轮已完成","demo-docs")]
        for i,(title,activity,project) in enumerate(entries):
            y = 196+i*102
            if detail and i == 0:
                self.rect((10,y-7,208,y+85),C["bluebg"],radius=7)
            self.text(20,y+5,title,14,bold=True,maxw=168)
            self.circle(25,y+37,3,C["amber"] if offline else (C["blue"] if i<2 else C["green"]))
            state = "上次：运行中" if offline and i<2 else "上次：已完成" if offline else (
                    "运行中 · "+activity if i<2 else activity)
            self.text(36,y+31,state,12,C["sub"],maxw=166)
            self.text(20,y+60,project,12,C["sub"])
        self.line([(14,718),(204,718)])
        self.icon("link",21,741,size=17)
        self.text(50,742,"连接诊断",14)
        self.circle(25,822,3.5,C["amber"] if offline else C["green"])
        self.text(38,815,"正在重新连接" if offline else "已连接 · 3 个会话",12,
                  C["amber"] if offline else C["green"])
        self.text(20,843,"保留上次状态" if offline else "原任务在 Codex 中继续",11,C["sub"])
        self.rect((0,872,W,900),"#F7F8FA")
        self.line([(0,872),(W,872)])
        self.text(16,881,"Codex Desktop",11,C["sub"])
        self.text(236,881,"缓存快照 · 等待同步" if offline else "消息与任务状态分别更新",11,C["sub"])
        self.text(1205,881,"静态设计稿 · 演示内容",11,C["sub"])

    def save(self, name, description):
        bad = [b for b in self.bounds if b[0]<0 or b[1]<0 or b[2]>W or b[3]>H]
        assert not bad, f"文本越界：{bad}"
        info = PngImagePlugin.PngInfo()
        info.add_text("Description", description)
        info.add_text("Provenance", "Deterministic vector and text drawing with Pillow; synthetic content; no real session messages.")
        self.image.save(OUT/name,pnginfo=info)


def message(page,x,y,width,who,at,body):
    page.text(x,y,who,13,C["blue"] if who=="Codex" else C["sub"],bold=True)
    page.text(x+width-58,y,at,11,C["sub"])
    return page.para(x,y+28,body,width,15,lineh=26)+16


def overview(offline=False):
    p = Page(); p.shell(offline=offline)
    start = 224
    if offline:
        p.rect((238,110,1420,188),C["amberbg"],radius=7)
        p.icon("warning",254,130,C["amber"],20)
        p.text(286,125,"监控连接已断开，正在重新连接",18,C["amber"],True)
        p.text(286,157,"消息停留在 12:06:18 的快照；任务当前状态尚未确认。",13,C["amber"])
        p.button(1274,131,"立即重试",120,icon="refresh")
        p.text(242,210,"会话总览",23,bold=True)
        p.text(242,249,"上次同步了 3 个会话 · 恢复连接后自动重新同步",13,C["sub"])
        start = 288
    else:
        p.text(242,118,"会话总览",24,bold=True)
        p.text(242,156,"3 个已有会话 · 2 个运行中，1 个本轮完成",14,C["sub"])
        p.button(1274,122,"重新扫描",120,icon="refresh")
        p.rect((242,184,546,214),"#E9EDF3",radius=5)
        p.rect((245,187,343,211),C["white"],radius=4)
        p.text(267,193,"全部 3",12,bold=True)
        p.text(363,193,"运行中 2",12,C["sub"])
        p.text(463,193,"已完成 1",12,C["sub"])
        p.icon("grid",1323,190,C["blue"],16)
        p.icon("stream",1371,190,C["sub"],16)

    specs = [
        ("检查会话同步","demo-core","检查多个会话更新时，状态是否会互相影响。",
         "正在核对消息的会话归属，并检查断线后的同步顺序。", "检查增量版本", "检查增量版本"),
        ("修复日志字段","demo-api","统一错误日志字段，保留现有调用方式。",
         "已找到字段不一致的调用点，正在检查对应测试。", "运行相关测试", "等待测试结果"),
        ("梳理模块依赖","demo-docs","整理模块依赖，说明主要调用关系。",
         "已完成依赖梳理。主要入口和三条调用路径已整理，可以继续查看结果。", "生成关系清单", "本轮任务已结束"),
    ]
    gap=12; width=384; left=242
    for i,(title,project,user,assistant,tool,activity) in enumerate(specs):
        x=left+i*(width+gap); end=822
        p.rect((x,start,x+width,end),C["white"],C["line"])
        p.text(x+18,start+20,title,17,bold=True,maxw=320)
        state="上次：运行中" if offline and i<2 else "上次：已完成" if offline else (
              "运行中" if i<2 else "本轮完成")
        bw=p.badge(x+18,start+55,state,"amber" if offline else "blue" if i<2 else "green")
        p.text(x+18+bw+13,start+63,"等待同步" if offline else "连接正常",12,C["sub"])
        p.text(x+18,start+94,project,12,C["sub"])
        p.line([(x,start+124),(x+width,start+124)])
        y=message(p,x+18,start+144,width-42,"你","11:54",user)
        p.line([(x+18,y-2),(x+width-18,y-2)])
        y=message(p,x+18,y+17,width-42,"Codex","11:55",assistant)
        p.rect((x+18,y,x+width-18,y+44),C["soft"],C["line"],5)
        p.icon("chevron",x+29,y+13,size=16)
        p.text(x+53,y+14,tool,13,maxw=200)
        tool_state=("上次：已完成" if i==2 else "上次：进行中") if offline else ("完成" if i==2 else "进行中")
        state_width=p.d.textlength(tool_state,font=font(11))/S
        p.text(x+width-22-state_width,y+15,tool_state,11,
               C["amber"] if offline else C["green"] if i==2 else C["blue"])
        if not offline and i==1:
            p.rect((x+18,y+58,x+width-18,y+124),C["code"],radius=4)
            p.text(x+30,y+71,"> cargo test log_fields",12,C["sub"],mono=True)
            p.text(x+30,y+95,"running 4 tests",12,C["sub"],mono=True)
        if not offline:
            p.text(x+18,690,"当前活动" if i<2 else "本轮结果",12,C["sub"])
            p.text(x+18,719,activity,14,bold=True,maxw=340)
        p.line([(x+18,760),(x+width-18,760)])
        p.text(x+18,779,"等待重新同步" if offline else "最新消息 · 刚刚" if i<2 else "最新回复 · 1 分钟前",12,C["sub"])
        p.text(x+width-113,779,"查看详情",12,C["blue"])
        p.icon("arrow",x+width-38,777,C["blue"],16)
        p.rect((x+width-7,start+135,x+width-4,end-75),"#F4F6F9",radius=1)
        p.rect((x+width-7,start+170,x+width-4,start+280),"#BBC5D2",radius=1)
    p.text(242,842,"每一栏独立滚动；打开会话详情可集中阅读。",12,C["sub"])
    p.save("03-reconnect.png" if offline else "01-overview.png",
           "原生桌面静态设计稿："+("断线重连，上次状态不是任务失败。" if offline else "三个已有会话的并排消息流。"))
    return p.image


def detail():
    p=Page();p.shell(detail=True)
    p.rect((219,93,1439,871),C["white"])
    p.text(248,118,"检查会话同步",24,bold=True)
    p.button(1232,120,"在 Codex 中查看",172,icon="arrow")
    bw=p.badge(248,157,"运行中")
    p.circle(248+bw+20,169,3,C["green"])
    p.text(248+bw+31,163,"连接正常 · 最近同步：刚刚",13,C["sub"])
    p.text(248,196,"demo-core / 当前会话",12,C["sub"])
    p.line([(219,225),(1440,225)])
    p.line([(1138,226),(1138,872)])
    p.rect((1139,226,1439,871),C["soft"])
    x=251;width=850
    p.text(x,253,"你",14,bold=True);p.text(x+width-65,255,"11:54:10",11,C["sub"])
    p.rect((x,284,x+width,335),C["canvas"],radius=5)
    p.text(x+16,302,"检查多个会话更新时，状态是否会互相影响。",15)
    p.text(x,367,"Codex",14,C["blue"],True);p.text(x+width-65,369,"11:54:22",11,C["sub"])
    p.para(x,399,"我会分别检查三个会话的消息和任务状态，确认它们独立更新，并核对重连后的同步顺序。",width,15,lineh=27)
    for j,(tool,status) in enumerate([("读取已有会话索引","已完成"),("检查增量版本","进行中")]):
        y=467+j*53
        p.rect((x,y,x+width,y+41),C["soft"],C["line"],5)
        p.icon("chevron",x+12,y+13,size=15)
        p.text(x+38,y+13,tool,13)
        p.text(x+width-68,y+14,status,12,C["green"] if j==0 else C["blue"])
    p.text(x,590,"Codex",14,C["blue"],True)
    p.text(x+72,592,"正在回复",12,C["sub"])
    p.para(x,622,"每个会话的消息和状态分别更新。监控连接中断时，界面保留上次内容；恢复后再重新同步，不会把断线显示成任务失败。",width,15,lineh=27)
    p.rect((x,701,x+width,777),C["code"],radius=5)
    p.text(x+15,715,"Session A  running",13,C["sub"],mono=True)
    p.text(x+15,740,"Session B  completed",13,C["sub"],mono=True)
    p.icon("copy",x+width-31,716,size=17)
    p.rect((219,817,1138,871),"#F8FAFD")
    p.line([(219,817),(1138,817)])
    p.icon("info",251,835,C["sub"],17)
    p.text(278,837,"在 Codex Desktop 中继续对话，这里同步显示消息。",13,C["sub"])
    p.button(964,828,"回到最新消息",148,icon="arrow")
    p.rect((1127,244,1131,796),"#F1F4F8",radius=2)
    p.rect((1127,573,1131,760),"#B6C1D0",radius=2)

    rx=1162
    p.text(rx,252,"任务状态",17,bold=True)
    p.badge(rx,289,"运行中")
    p.text(rx,334,"当前活动",12,C["sub"])
    p.text(rx,361,"检查增量版本",15,bold=True)
    p.text(rx,410,"最近活动",13,bold=True)
    activities=[("发现已有会话","已完成 · 11:54",True),
                ("读取初始消息","已完成 · 11:54",True),
                ("检查增量版本","进行中 · 刚刚",False)]
    p.line([(rx+8,454),(rx+8,579)],"#CDD8E7",1)
    for i,(title,sub,done) in enumerate(activities):
        y=445+i*62
        p.circle(rx+8,y+8,8,C["greenbg"] if done else C["bluebg"])
        if done:p.icon("check",rx+1,y+1,C["green"],14)
        else:p.circle(rx+8,y+8,3,C["blue"])
        p.text(rx+28,y,title,13)
        p.text(rx+28,y+25,sub,11,C["sub"])
    p.line([(rx,632),(1415,632)])
    p.text(rx,656,"监控连接",15,bold=True)
    p.circle(rx+4,694,3.5,C["green"])
    p.text(rx+17,687,"已连接",13,C["green"])
    p.text(rx,720,"本机 · Codex Desktop",12,C["sub"])
    p.text(rx,748,"最近同步：刚刚",12,C["sub"])
    p.text(rx,796,"原任务继续在 Codex 中运行。",12,C["sub"])
    p.save("02-detail.png","原生桌面静态设计稿：会话详情、原生消息区和独立活动状态。")
    return p.image


if __name__ == "__main__":
    images=[overview(),detail(),overview(True)]
    thumbs=[]
    for image in images:
        thumbs.append(image.resize((864,540),Image.Resampling.LANCZOS))
    sheet=Image.new("RGB",(864,1668),"#E9EDF3")
    for i,image in enumerate(thumbs):sheet.paste(image,(0,i*564))
    sheet.save(OUT/"review-sheet.png")
    print(json.dumps({"pages":["01-overview.png","02-detail.png","03-reconnect.png"],
                      "size":[W*S,H*S],"formalUiCode":False,"realUserContent":False}))
