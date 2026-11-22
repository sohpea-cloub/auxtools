// Guards: the code of a reduced fuzz case: reload mode on 516.1688, seed 842, "new: object's vars differ"
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
	var/vn8 = 100
	var/vt9 = "q\"q"
	var/list/vl10
	var/va11 = 100
	var/datum/fz/s3/vo12
	var/vn13 = 2.5

/datum/fz/s3
	var/list/vl14
	var/vn16 = 1e3

/proc/fz_setup()

/proc/fz_fields(datum/d)
	if (d.type == /datum/fz/s3)
		var/datum/fz/s3/x1 = d
		return "vn8=[fz_show(x1.vn8, 2)] vt9=[fz_show(x1.vt9, 2)] vl10=[fz_show(x1.vl10, 2)] va11=[fz_show(x1.va11, 2)] vo12=[fz_show(x1.vo12, 2)] vn13=[fz_show(x1.vn13, 2)] vl14=[fz_show(x1.vl14, 2)] vn16=[fz_show(x1.vn16, 2)]"

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(174, /datum/fz/s3, "t168", 0)

/datum/fz/s3/proc/t168(a, b = 2, c)
	src.vn8 = text2ascii_char("\xff\x41", 2)
