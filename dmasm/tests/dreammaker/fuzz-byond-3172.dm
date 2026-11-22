// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 3172, "515.1637: g: result differs"
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
	fz_call(78, "g", null, "gt76")

/proc/gt76(a, b = 2, c)
	try
		throw EXCEPTION("")
	catch (var/exception/xx77)
		. = list(., istype(xx77) ? list(xx77.name, xx77.file, xx77.line, xx77.desc) : xx77)
