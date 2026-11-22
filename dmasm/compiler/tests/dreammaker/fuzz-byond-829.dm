// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 829, "515.1637: new: object's vars differ"
#define DEBUG

#define FZD3 10
#define FZM7 2

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
	var/static/vn6 = 0x1F
	var/vt7 = "A,b,c"
	var/list/vl8
	var/tmp/va9 = null
	var/static/datum/fz/vo10
	var/vn11 = 255
	var/const/vn12 = 2
	var/list/vl13
	var/static/list/vl14 = list("x", "y", "z")

/datum/fz/s4
	var/vt15 = "1"
	var/list/vl16

/datum/fz/s5
	var/vt17 = "q\"q"
	var/list/vl18 = list("x", "y", "z")
	var/vn19 = 0.5

var/list/gl20 = list("x", "y", "z")
var/gn21
var/gn22 = 16777215
var/ga23 = -3.25
var/gt24 = "12abc"
var/gn25 = 100

/proc/fz_setup()

/proc/fz_fields(datum/d)
	if (d.type == /datum/fz)
	if (d.type == /datum/fz/s4)
		var/datum/fz/s4/x1 = d
		return "vn6=[fz_show(x1.vn6, 2)] vt7=[fz_show(x1.vt7, 2)] vl8=[fz_show(x1.vl8, 2)] va9=[fz_show(x1.va9, 2)] vo10=[fz_show(x1.vo10, 2)] vn11=[fz_show(x1.vn11, 2)] vn12=[fz_show(x1.vn12, 2)] vl13=[fz_show(x1.vl13, 2)] vl14=[fz_show(x1.vl14, 2)] vt15=[fz_show(x1.vt15, 2)] vl16=[fz_show(x1.vl16, 2)]"
	if (d.type == /datum/fz/s5)
		var/datum/fz/s5/x2 = d

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(151, /datum/fz/s4, "t147", 1)
	return

/proc/gh28()
	var/loops = 0
	var/n1 = 0.5
	var/s1 = "1"
	var/list/L = list(7, "1")
	var/v
	for (var/xe29 as anything in L)
	for (var/e30 in splittext(json_encode(gn21), ","))
		gn21--
	return ((gn21 || text2path("/datum/nope")) ? 2 ~! 1 : (text2num(s1) ? L : L) ? gl20 : null)

/datum/fz/proc/h34()
	var/loops = 0
	var/n1 = 16777215
	return ("[vl14])" ~! "Hello")

/datum/fz/s5/proc/h41(x1 = null, x2 = 10)
	var/loops = 0
	var/n1 = 5
	var/s1 = "UPPER lower"
	var/list/L = list(65535, "Hello")
	var/v
	switch (gh28())
		if (1)
			switch (gn21--)
				if (5 to 10)
					gt24 = "Hello"
		if ("abc")
			if (h34())
				vn6--
				n1 |= min(max(((3 ~! n1) %% 10), -16777216), 16777215)
	for (var/i44 in 1 to 1)
		if (++loops > 40) break
		if ((src.vo10?.h34() ~! vn19))
			vl14 += "x"
		try
			var/xz46 = 1 / (loops * 0)
		catch (var/exception/xx46)
			. = list(., istype(xx46) ? list(xx46.name, xx46.file, xx46.line, xx46.desc) : xx46)
	return n1

/datum/fz/s4/proc/t147(a, b = 2, c)
	var/loops = 0
	var/n1 = 3
	var/s1 = "  pad  "
	var/n2 = 10
	var/list/L = list(10, "k=1&j=2", 2.5)
	var/list/M = list("a" = 3, "b" = "A,b,c")
	var/datum/fz/s5/o = new /datum/fz/s5(1e3, "3.5")
	var/datum/fz/p = new /datum/fz
	var/v
	o.h41((b ? vl16 : src), 7)
	. = list(., "[vo10]\he \A [vl13] ")
	if (sorttextEx(gh28(), "  pad  ")) return text2num((gt24 + vt7))
	do
		for (var/e150 in list())
		src.vl14 += (vn6 ? length(gl20) : gn22)
	while (0.5)
	return list(a, b, c, n1, n2, s1, L, M, o, p, v, ., loops)
