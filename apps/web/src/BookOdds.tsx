import { useRef, useState } from 'react';
import { formatTwoWayMoneyline, parseMoneyline, twoWayExpectedValue } from './model.ts';

type Side = { name: string; fair: number };

/** Expected profit per $100 staked, to the cent, signed. */
function perHundred(ev: number) {
  const cents = Math.round(10000 * ev);
  return `${cents > 0 ? '+' : cents < 0 ? '-' : ''}$${(Math.abs(cents) / 100).toFixed(2)}`;
}

/**
 * Opens a dialog that prices the book's moneylines against the model's fair two-way lines. The parent keys it by
 * matchup, so lines entered for one game never carry over to another.
 */
export function BookOdds({ home, away, tie }: { home: Side; away: Side; tie: number }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [lines, setLines] = useState({ home: '', away: '' });
  const sides = [
    ['home', home],
    ['away', away],
  ] as const;
  return (
    <>
      <button className="book-odds-open" onClick={() => dialog.current?.showModal()}>
        Compare book odds
      </button>
      <dialog ref={dialog} className="book-odds" aria-labelledby="book-odds-heading">
        <p className="eyebrow">Expected value</p>
        <h2 id="book-odds-heading">Compare book odds</h2>
        <p className="muted">
          Enter the sportsbook's moneylines as quoted, vig included. Expected value uses the book's payout and assumes Rally Row's
          two-way moneylines are exact. A tie is a push.
        </p>
        {sides.map(([key, side]) => {
          const text = lines[key];
          const line = parseMoneyline(text);
          const ev = line === null ? null : twoWayExpectedValue(side.fair, line, tie);
          const id = `book-odds-${key}`;
          return (
            <div className="book-odds-row" key={key}>
              <label htmlFor={id}>{side.name}</label>
              <small>Fair two-way ML {formatTwoWayMoneyline(side.fair)}</small>
              <input
                id={id}
                autoComplete="off"
                spellCheck={false}
                placeholder="-110"
                value={text}
                onChange={(e) => setLines((current) => ({ ...current, [key]: e.target.value }))}
              />
              <output htmlFor={id} className={ev !== null && ev > 0 ? 'positive' : undefined}>
                {text.trim() === ''
                  ? 'Enter the book’s line'
                  : ev === null
                    ? 'Enter +100 or higher, or −100 or lower'
                    : `EV ${perHundred(ev)} per $100`}
              </output>
            </div>
          );
        })}
        <button className="book-odds-close" onClick={() => dialog.current?.close()}>
          Done
        </button>
      </dialog>
    </>
  );
}
