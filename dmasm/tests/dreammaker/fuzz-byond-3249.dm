// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 3249, "515.1647: g: result differs"
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
	fz_call(96, "g", null, "gt86")

/datum/fz/s4/proc/h36()

/proc/gt86(a, b = 2, c)
	var/datum/fz/s4/p = new /datum/fz/s4(255, "k=1&j=2")
	try
		switch (round(0.1, -1))
			if (0)
				. = list(., p.h36())
	catch (var/x89)
