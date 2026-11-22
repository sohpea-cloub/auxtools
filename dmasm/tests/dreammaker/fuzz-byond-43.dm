// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 43, "515.1647: new: runtime error only here: list index out of bounds"
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

/datum/fz/s3/s4
	var/list/vl14 = list()

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(64, /datum/fz/s3/s4, "t50", 1)

/datum/fz/s3/s4/proc/h21()
	return vl14[1]

/datum/fz/s3/proc/t50(a, b = 2, c)

/datum/fz/s3/s4/t50(a, b = 2, c)
	spawn (-1)
		fz_log("s62", h21())
