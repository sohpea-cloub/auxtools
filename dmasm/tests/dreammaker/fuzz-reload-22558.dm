// Guards: the code of a past finding: reload mode on 515.1637, seed 22558, "reloaded world gave no results (exited): BUG: Crashing due to an illegal operation!"
#define DEBUG

#define FZD2 1

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
	var/list/vl13 = list(1, 2)

/datum/fz/s6

var/gn21 = 2

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(113, /datum/fz/s3/s4/s5, "t110", 1)

/proc/gh37(x1 = null, x2, x3)

/datum/fz/s3/proc/h44(x1, x2 = "")
	var/loops = 0
	var/n1 = 256
	var/s1 = "3.5"
	var/list/L = list(1, "  pad  ")
	var/v
	src.vl13["b"] ||= gh37((args.Copy() + null), text2path("/datum/nope"), src.vl13["k"])
	var/list/l45 = (L - (min(max(gn21, -16777216), 16777215) << 2))

/datum/fz/s3/s4/s5/proc/t110(a, b = 2, c)
	var/datum/fz/s3/s4/s5/o = new /datum/fz/s3/s4/s5(0.001, "k=1&j=2")
	var/datum/fz/s6/p = new /datum/fz/s6
	var/v
	if (isloc((v || FZD2)))
	else
		o.h44(findlasttextEx({"multi "quoted" text"}, "a\tb"), "[p]\t[null])")
