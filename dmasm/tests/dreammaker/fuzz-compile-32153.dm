// Guards: the code of a past finding: compile mode on 515.1647, seed 32153, "DreamMaker's world gave no results (1): BUG: Crashing due to an illegal operation!"
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
	var/tmp/vn9 = -4
	var/static/list/vl11 = list("x", "y", "z")
	var/datum/fz/vo13
	var/vn14 = 255

/obj/fz
	var/vt20

var/gn25 = 1
var/list/gl26
var/gn27 = 0.001

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(89, /obj/fz/s8, "vb82", 1)
	fz_case(169, /datum/fz/s4, "t159", 1)

/datum/fz/s4/s6/proc/h41()

/obj/fz/proc/h45(x1, x2, x3)
	var/list/L = list(-3.25, "a b")
	switch ("[list(1, 2, 3)])")
		else
			if (rgb(b = 16, r = 16, g = clamp(round(length(gl26)), 0, 255), a = clamp(round(sqrt(abs(gn25))), 0, 255))) return L

/obj/fz/proc/h52()

/obj/fz/s8/h52()
	return (gn27 + gn25)

/obj/fz/s8/verb/vb82(a as num, b as num, c as text)
	var/datum/fz/o = new /datum/fz(7)
	if (src.h52())
		for (var/i86 = 1 to 1e3 step 3)
			gn25 = ((bounds_dist(src, o) * isnum(o.vl11)) + (~o::vn14))

/datum/fz/proc/t159(a, b = 2, c)
	var/list/M = list("a" = 1e3, "b" = "k=1&j=2")
	var/obj/fz/s8/o = new /obj/fz/s8
	o.vt20 = lowertext(o.h45(M, vo13?.vn9, copytext("  pad  ", 3)))
