// Guards: the code of a reduced fuzz case: reload mode on 516.1688, seed 971, "new: result differs"
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

/mob/fz/s8

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(126, /mob/fz/s7, "t123", 1)
	return

/obj/fz/s6

/mob/fz/proc/t123(a, b = 2, c)

/mob/fz/s7/t123(a, b = 2, c)
	var/loops = 0
	try
		var/xz124 = 1 / (loops * 0)
	catch (var/exception/xx124)
		. = list(., istype(xx124) ? list(xx124.name, xx124.file, xx124.line, xx124.desc) : xx124)
