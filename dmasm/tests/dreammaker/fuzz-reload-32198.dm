// Guards: the code of a past finding: reload mode on 515.1637, seed 32198, "g: result differs"
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
	var/tmp/vn6 = 2
	var/const/vt7 = "A,b,c"
	var/list/vl8 = list("a" = 1, "b" = 2)
	var/static/va9
	var/static/datum/fz/s5/vo10
	var/vn11 = 7
	var/const/vn12 = 1e3
	var/va13
	var/vn14 = 10

/datum/fz/s5
	var/static/vt15 = "12abc"
	var/static/vn16 = 256
	var/list/vl17 = list()

var/gt20
var/gn21 = 3

/proc/fz_setup()

/proc/fz_fields(datum/d)
	if (d.type == /datum/fz/s5)
		var/datum/fz/s5/x2 = d
		return "vn6=[fz_show(x2.vn6, 2)] vl8=[fz_show(x2.vl8, 2)] va9=[fz_show(x2.va9, 2)] vo10=[fz_show(x2.vo10, 2)] vn11=[fz_show(x2.vn11, 2)] vn12=[fz_show(x2.vn12, 2)] va13=[fz_show(x2.va13, 2)] vn14=[fz_show(x2.vn14, 2)] vt15=[fz_show(x2.vt15, 2)] vn16=[fz_show(x2.vn16, 2)] vl17=[fz_show(x2.vl17, 2)]"
	return ""

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_call(55, "g", null, "gt51")

/datum/fz/s5/s118/proc/t122(a, b = 2, c)
	if (src.h46())
		if (gt20)
		else if (1e3)
			gn21 ^= min(max(findlasttextEx((src.vt7 + "q\"q"), "abc"), -16777216), 16777215)

/datum/fz/s5/proc/h46()

/proc/gt51(a, b = 2, c)
	var/loops = 0
	var/n1 = 2
	var/s1 = "q\"q"
	var/n2 = 16777215
	var/list/L = list(16777215, "", 2.5)
	var/list/M = list("a" = 2.5, "b" = "12abc")
	var/datum/fz/s5/o = new(65535, "12abc")
	var/datum/fz/s5/p = new(0.5, "12abc")
	var/v
	p.vt15 = o.vt7
	return list(a, b, c, n1, n2, s1, L, M, o, p, v, ., loops)
