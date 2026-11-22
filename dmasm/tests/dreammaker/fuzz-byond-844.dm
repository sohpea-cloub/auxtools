// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 844, "515.1647: new: result differs"
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

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(132, /datum/fz/s6, "t122", 1)

/datum/fz/s6/proc/t122(a, b = 2, c)
	var/list/M = list("a" = -4, "b" = "")
	for (var/e127 as anything in M)
		try
			if (cos(-90)) throw null
		catch (var/x128)
			. = list(., istype(x128, /exception) ? "exception" : x128)
