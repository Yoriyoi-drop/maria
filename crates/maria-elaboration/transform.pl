#!/usr/bin/perl
use strict;
use warnings;

# This script transforms Err format/string returns and Result types in elaborator.rs
# Strategy: process character-by-character for format! wrapping, use regex for simpler patterns.

local $/;
my $text = <>;

# 1. Add import
$text =~ s/^(use super::util::\*;)\n/$1\nuse crate::error::SimError;\n/m;

# 2. Replace Err("static".to_string()) -> Err(SimError::elaborate("static"))
$text =~ s/\bErr\("([^"]*)"\)\.to_string\(\)/Err(SimError::elaborate("$1"))/g;

# 3. Replace Err("string")\n patterns - where there's a .to_string() on same line
# (already handled above)

# 4. Handle Err(format!(...)) -> Err(SimError::elaborate(format!(...)))
# We need to count parens to find the correct closing.
# Strategy: scan character by character, track state.

my $out = "";
my $pos = 0;
my $len = length($text);
my $depth = 0;
my $in_string = 0;
my $in_format = 0;        # depth of format! macro nesting
my $fmt_paren_depth = 0;  # paren depth to match format! closing
my $fmt_start_pos = 0;    # position in $out where format! opening paren was placed
my $state = 0;
# state 0: normal
# state 1: have seen "Err(format!(" and now tracking depth

while ($pos < $len) {
    my $ch = substr($text, $pos, 1);

    # Check for Err(format!( pattern
    if ($state == 0 && $pos + 15 <= $len && substr($text, $pos, 15) =~ /^Err\(format!\($/) {
        $out .= "Err(SimError::elaborate(format!(";
        $state = 1;
        $fmt_paren_depth = 1; # format!( adds depth 1
        $fmt_start_pos = length($out); # not really needed
        $pos += 15;
        next;
    }

    if ($state == 1) {
        $out .= $ch;
        if ($ch eq '"' && $pos > 0 && substr($text, $pos-1, 1) ne '\\') {
            $in_string = !$in_string;
        }
        if (!$in_string) {
            if ($ch eq '(') {
                $fmt_paren_depth++;
            } elsif ($ch eq ')') {
                $fmt_paren_depth--;
                # When depth drops to 0, we've closed format! and the outer Err
                # Original: format!(...)) means depth 0 = format! close, then Err close
                # But we added SimError::elaborate( so we need one more )
                # When fmt_paren_depth goes to 0 for the SECOND time, that's the Err close
                # Actually, the original has: format!( ... ) ) 
                # After: Err( SimError::elaborate( format!( ... ) ) )
                # Parent depth after format! close: 2 (Err + elaborate still open)
                # Parent depth after elaborate close: 1 (Err still open)
                # Parent depth after Err close: 0
                # So we need the depth to go 1→0→1→0 (format! close → elaborate close → Err close)
                # But tracking this is tricky.

                # Simpler view: original had format!( ... )) where ) closes format! and ) closes Err.
                # After adding SimError::elaborate(: original closing ) for elaborate doesn't exist.
                # We need ) for format! + ) for elaborate + ) for Err.
                # Original had 2 ) at closing, now we need 3.
                # So when we've found 2 consecutive ) that close the whole thing, make it 3.
                # But how to identify these reliably?
                
                # The original Err(format!(...)) has balanced parens.
                # The format!(...)) has depth: 1 (after format!(), depth increases when we encounter ( inside args, decreases with )).
                # When all parens inside format! are balanced, depth returns to 1 (the format!( itself).
                # Then ) closes format! → depth 0.
                # Then ) closes Err → depth -1? No, Err adds one level.
                # Original: Err( format!( ... ) ) 
                # After format!( : depth = 1 (the format!'s paren)
                # After args/balanced parens: depth should be 1
                # After first ) closing format!: depth = 0
                # After second ) closing Err: depth = -1
                
                # After transformation: Err( SimError::elaborate( format!( ... ) ) )
                # After format!( : depth = 1
                # After args: depth = 1
                # After first ) close format!: depth = 0
                # After second ) close elaborate: depth = -1
                # After third ) close Err: depth = -2
                
                # So when depth goes from 1 to 0, that's format! close.
                # Then depth goes from 0 to -1, that's the old Err close (now elaborate close).
                # We need ONE MORE depth transition at the end.
                
                # After the original had depth -1 (Err close), that was the end.
                # Now after the replacement, at that same point depth is 0 (not -1).
                # Because elaborate( added one more open.
                
                # So the strategy: track depth. When it reaches -1 at the end of the Err call,
                # that means we're done. But now after inserting SimError::elaborate(,
                # at the original Err close position the depth is 0, not -1.
                # We add one more ) to make it -1.
                
                # Actually it's even simpler. Original has:
                # Err( format!( ... ) )
                # Depth: after format!( = 1, after args + inner parens balanced = 1,
                # after ')' = 0 (format! close), after final ')' = -1 (Err close)
                
                # After inserting SimError::elaborate(:
                # Depth: after format!( = 1, after args = 1, after ')' = 0 (format! close),
                # at the point of original second ')', depth was -1, but now we added elaborate(,
                # so depth at that point is 0. We need one more ')'.
                
                # Let me track: when depth was 0 after format! close in the ORIGINAL, 
                # we saw the next ) to close Err making depth -1.
                # Now depth is 0 after format! close, next ) closes elaborate making depth -1,
                # and we need ANOTHER ) to close Err making depth -2.
            }
        }
        $pos++;
        next;
    }

    $out .= $ch;
    $pos++;
}

# OK this is getting too complex. Let me take a completely different approach.

print $text;
