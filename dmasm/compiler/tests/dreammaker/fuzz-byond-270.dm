// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 270, "515.1647: new: another runtime error: type mismatch: "…" %= null, not type mismatch: "…" %= N"
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
	var/vn7
	var/tmp/list/vl9 = list(1, 2)

var/gn19 = 0.5

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(84, /datum/fz/s5/s6, "t76", 1)

/datum/fz/s5/s6/proc/h29(x1, x2)

/datum/fz/proc/h44()

/datum/fz/s5/h44()
	src.vn7 %= 2
	src.vn7 ||= (gn19++ && "A,b,c")

/datum/fz/proc/t76(a, b = 2, c)
	var/datum/fz/s5/s6/o = new /datum/fz/s5/s6(10)
	for (var/e77 as anything in o.vl9)
		h44()
