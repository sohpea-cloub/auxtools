// Guards: the code of a reduced fuzz case: reload mode on 516.1688, seed 66, "new: result differs"
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
	var/static/vn9
	var/static/list/vl11 = list(1, 2)
	var/tmp/va12
	var/datum/fz/s5/vo13
	var/vn14
	var/static/vn15

/datum/fz/s5/s6/s105

/proc/fz_setup()

/proc/fz_fields(datum/d)
	if (d.type == /datum/fz)
		var/datum/fz/x0 = d
		return "vn9=[fz_show(x0.vn9, 2)] vl11=[fz_show(x0.vl11, 2)] va12=[fz_show(x0.va12, 2)] vo13=[fz_show(x0.vo13, 2)] vn14=[fz_show(x0.vn14, 2)] vn15=[fz_show(x0.vn15, 2)]"

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(89, /datum/fz/s5/s8, "t85", 1)
	fz_case(117, /datum/fz/s5/s6/s105, "t80", 0)

/obj/fz/proc/h46()

/datum/fz/s5/proc/t80(a, b = 2, c)
	var/loops = 0
	var/n1 = 0x1F
	var/s1 = "abc"
	var/n2 = -3.25
	var/list/L = list(0x1F, "a\tb", 255)
	var/list/M = list("a" = 0, "b" = "abc")
	var/datum/fz/o = new /datum/fz
	var/obj/fz/p = new /obj/fz
	var/v
	return list(a, b, c, n1, n2, s1, L, M, o, p, v, ., loops)

/datum/fz/s5/s8/proc/t85(a, b = 2, c)
	vl11 += "k=1&j=2"
