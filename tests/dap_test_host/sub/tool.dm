/proc/sub_tool(n)
	return n + 1 // @sub_tool

/proc/sub_declares(n)
	var/unset // @declares_var
	n += 1 // @declares_first
	return n + length(unset) // @declares_second

var/tooled = sub_tool(20) // @tooled
