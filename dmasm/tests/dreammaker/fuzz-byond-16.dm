// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 16, "515.1647: g: runtime error only here: type mismatch: cannot compare N to "…""
#define DEBUG

#ifdef FZD_ONE
#endif

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

/datum/fz/s3
	var/vt13 = "q\"q"

/obj/fz
	var/tmp/vt17 = "Hello"

var/gn23 = 2
var/gn25
var/gt26 = "k=1&j=2"

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_call(101, "g", null, "gt99")

/obj/fz/s6/proc/h32(x1, x2 = null, x3)
	var/list/L = list(0.5, "k=1&j=2")
	gn25 = max(L)

/datum/fz/proc/h58(x1, x2, x3)
	try
		if ("["A,b,c"]") throw list(1, 2)
	catch (var/x59)

/proc/gt99(a, b = 2, c)
	var/list/L = list(10, "q\"q", 3)
	var/obj/fz/s6/o = new /obj/fz/s6(null, 5, "UPPER lower")
	var/datum/fz/s3/p = new
	if ((p.h58(L, list(p, gn25, o.vt17), call(o, /obj/fz/s6/proc/h32)(o, L, sorttextEx(gt26, p.vt13))) + gn23))
		return
