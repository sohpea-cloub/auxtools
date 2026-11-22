// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 58, "516.1659: g: result differs"
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

/datum/fz/s5

/obj/fz
	var/vt20 = "Hello"

var/list/gl28

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_call(181, "g", null, "gt174")

/proc/gt174(a, b = 2, c)
	var/list/L = list(-3.25, "q\"q", 0)
	var/obj/fz/p = new /obj/fz
	if (p.vt20)
		if (get_dist(new /datum/fz/s5(0.5, rgb(180.5, 100, 0, space = COLORSPACE_HSL)), p)) return (L ^ gl28)
