// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 86, "516.1688w: new: object's vars differ"
#define DEBUG

var/fz_news = 0
var/fz_budget = 0
var/list/fz_out = list()
var/list/fz_logs = list()
var/list/fz_old = list()
/proc/fz_show(v, depth = 0)
/proc/fz_log(tag, v)
/proc/fz_line(t)
/proc/fz_sort(list/l)
/proc/fz_call(id, dom, datum/d, name)
/proc/fz_case(id, t, name, old)
/proc/fz_ti(t, list/names, list/procs, datum/old)
/proc/fz_main()
/world/New()

/datum/fz
	var/vn10 = -3.25
	var/static/vt11 = "a\tb"
	var/list/vl12
	var/static/va13
	var/datum/fz/s5/s7/vo14
	var/vn15 = 0.5
	var/tmp/vt16 = "q\"q"
	var/static/vt17 = "x"

/datum/fz/s5/s7/s8
	var/tmp/vt18
	var/static/list/vl19

/obj/fz

/obj/fz/s9
	name = "thing 25"

var/gn26 = 100
var/list/gl27 = list("a" = 1, "b" = 2)
var/gn28 = 2.5
var/ga29
var/gt30

/proc/fz_setup()
	return

/proc/fz_fields(datum/d)
	if (d.type == /datum/fz)
		var/datum/fz/x0 = d
		return "vn10=[fz_show(x0.vn10, 2)] vt11=[fz_show(x0.vt11, 2)] vl12=[fz_show(x0.vl12, 2)] va13=[fz_show(x0.va13, 2)] vo14=[fz_show(x0.vo14, 2)] vn15=[fz_show(x0.vn15, 2)] vt16=[fz_show(x0.vt16, 2)] vt17=[fz_show(x0.vt17, 2)]"
	if (d.type == /datum/fz/s5)
		var/datum/fz/s5/x1 = d
		return "vn10=[fz_show(x1.vn10, 2)] vt11=[fz_show(x1.vt11, 2)] vl12=[fz_show(x1.vl12, 2)] va13=[fz_show(x1.va13, 2)] vo14=[fz_show(x1.vo14, 2)] vn15=[fz_show(x1.vn15, 2)] vt16=[fz_show(x1.vt16, 2)] vt17=[fz_show(x1.vt17, 2)]"
	if (d.type == /datum/fz/s6)
		var/datum/fz/s6/x2 = d
		return "vn10=[fz_show(x2.vn10, 2)] vt11=[fz_show(x2.vt11, 2)] vl12=[fz_show(x2.vl12, 2)] va13=[fz_show(x2.va13, 2)] vo14=[fz_show(x2.vo14, 2)] vn15=[fz_show(x2.vn15, 2)] vt16=[fz_show(x2.vt16, 2)] vt17=[fz_show(x2.vt17, 2)]"

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(98, /datum/fz, "t94", 1)
	return

/datum/fz/s6/proc/h40(x1 = "k=1&j=2")
	if (src)
	try
		if ((!va13)) throw (vn10 + gn28)
	catch
		gn26++
	return gn28

/datum/fz/proc/t94(a, b = 2, c)
	var/loops = 0
	var/n1 = 10
	var/s1 = "a\tb"
	var/n2 = 65535
	var/list/L = list(256, "  pad  ", 1e3)
	var/list/M = list("a" = 256, "b" = "a\tb")
	var/datum/fz/o = new
	var/datum/fz/s5/p = new /datum/fz/s5
	var/v
	var/p95 = &vn15
	*p95 = arccos((loops * 0 + -2))
	. = list(., *p95)
	for (var/i96 in 0 to (min(max(-3.25, -16777216), 16777215) << 2))
		if ((i96 || "1"))
			p.vt11 = "n[o.vn10--]"
		else if (p.vn15)
			v = (0 * -3.25)
	src.vt17 = "Hello"
	. = L["k"]
	fdel("fzf97.txt")
	text2file("[b]", "fzf97.txt")
