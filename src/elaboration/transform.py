#!/usr/bin/env python3
"""Transform elaborator.rs: String errors -> SimError"""
import re

with open('/home/whale-d/maria/src/elaboration/elaborator.rs', 'r') as f:
    text = f.read()

# 1. Add import
text = text.replace(
    'use super::util::*;\n',
    'use super::util::*;\nuse crate::error::SimError;\n',
    1
)

# ========== Helper: insert SimError::elaborate(...) wrapping a format!(...) ==========
def wrap_format_call(text, prefix_before_format):
    """
    Find `prefix_before_format + "format!("` in text, then track balanced parens
    to find the closing `))` (format! close + outer call close).
    Insert `SimError::elaborate(` after the opening; add extra `)` at closing.
    Returns transformed text.
    """
    marker = prefix_before_format + "format!("
    result = []
    i = 0
    n = len(text)
    marker_len = len(marker)

    while i < n:
        if (i + marker_len <= n and text[i:i+marker_len] == marker):
            # Append prefix + SimError::elaborate( + format!(
            result.append(prefix_before_format)
            result.append("SimError::elaborate(format!(")
            i += marker_len

            # Track paren depth from 1 (the format!( itself)
            depth = 1
            in_string = False
            while i < n:
                c = text[i]
                if c == '"' and (i == 0 or text[i-1] != '\\'):
                    in_string = not in_string

                if not in_string:
                    if c == '(':
                        depth += 1
                    elif c == ')':
                        depth -= 1
                        if depth == 0:
                            # This ) was the original format! close
                            result.append(')')  # close format!
                            i += 1  # skip original format! close )
                            # Next ) was the original outer call close
                            # Now: one for elaborate, one extra for outer call
                            if i < n and text[i] == ')':
                                result.append(')')  # close elaborate
                                i += 1
                                result.append(')')  # close outer (the extra one)
                            break

                result.append(c)
                i += 1
        else:
            result.append(text[i])
            i += 1

    return ''.join(result)


# 2. Handle Err(format!(...)) -> Err(SimError::elaborate(format!(...)))
text = wrap_format_call(text, "Err(")

# 3. Handle Err("...".to_string()) -> Err(SimError::elaborate("..."))
text = re.sub(
    r'\bErr\("([^"]*)"\)\.to_string\(\)',
    r'Err(SimError::elaborate("\1"))',
    text
)

# 4. Handle ok_or_else(|| format!(...)) -> ok_or_else(|| SimError::elaborate(format!(...)))
text = wrap_format_call(text, "ok_or_else(|| ")

# 5. Handle ok_or_else(|| "...".to_string()) -> ok_or_else(|| SimError::elaborate("..."))
text = re.sub(
    r'ok_or_else\(\|\|\s*"([^"]*)"\s*\)',
    r'ok_or_else(|| SimError::elaborate("\1"))',
    text
)

# 6. Replace Result<..., String> with Result<..., SimError>
def fix_result_types(text):
    result = []
    i = 0
    n = len(text)
    while i < n:
        if (i + 7 < n and text[i:i+7] == 'Result<'):
            start = i
            i += 7
            depth = 1
            first_type_end = None
            while i < n and depth > 0:
                if text[i] == '<':
                    depth += 1
                elif text[i] == '>':
                    depth -= 1
                    if depth == 0:
                        first_type_end = i  # position of the matching >
                i += 1
            # i now points past the matching >
            # Check if it's followed by , String>
            if first_type_end is not None:
                rest = text[first_type_end+1:i]
                rest_stripped = rest.strip()
                # Check if we have ", String>" pattern
                # rest should be ", String" followed by optional whitespace and >
                m = re.match(r'^\s*,\s*String\s*>\s*$', rest)
                if m:
                    first_type = text[start+7:first_type_end]
                    result.append(f"Result<{first_type}, SimError>")
                    continue  # i already advanced past ", String>"

            # Not a match/replacement, append original
            result.append(text[start:i])
        else:
            result.append(text[i])
            i += 1
    return ''.join(result)

text = fix_result_types(text)

# 7. Handle remaining ok_or_else patterns with .to_string()
text = re.sub(
    r'ok_or_else\(\|\|\s*"([^"]*)"\s*\.to_string\(\)\s*\)',
    r'ok_or_else(|| SimError::elaborate("\1"))',
    text
)

# 8. Handle ok_or_else(|| err_var) where err_var is a String expression
# After step 6, ok_or_else closures need to return SimError
# The original ok_or_else(|| expr) where expr returns String needs -> ok_or_else(|| SimError::elaborate(expr))
# But this is tricky to detect automatically. Let me check what remains.
# Handle the specific pattern: ok_or_else(|| format!(...)) where the closing already handled
# Also handle ok_or_else(|| expr) where expr is not just a format! or string literal

with open('/home/whale-d/maria/src/elaboration/elaborator.rs', 'w') as f:
    f.write(text)

print("Transform complete")
