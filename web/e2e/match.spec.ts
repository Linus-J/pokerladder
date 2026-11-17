import { expect, test, type Page } from '@playwright/test'

// Two browsers sign up, queue, and play a whole match through the UI.

const run = Date.now().toString(36).slice(-6)

async function signUp(page: Page, name: string) {
  await page.goto('/register')
  await page.getByLabel('Display name').fill(name)
  await page.getByLabel('Email').fill(`${name}@example.com`)
  await page.getByLabel('Password').fill('correct horse battery')
  await page.getByRole('button', { name: 'Create account' }).click()
  await expect(page.getByTestId('registered')).toContainText('Check your email')
  await page.getByTestId('registered').getByRole('link', { name: 'log in' }).click()
  await page.getByLabel('Name or email').fill(name)
  await page.getByLabel('Password').fill('correct horse battery')
  await page.getByRole('button', { name: 'Log in' }).click()
  await expect(page.getByRole('heading', { name: `Welcome back, ${name}` })).toBeVisible()
}

/** Acts whenever it's this player's turn: shove, or call a shove. */
async function shoveWhenAsked(page: Page) {
  const bar = page.getByTestId('action-bar')
  const allIn = bar.getByRole('button', { name: 'All-in', exact: true })
  const raise = bar.locator('button.raise')
  const call = bar.locator('button.call')
  if (await allIn.isVisible().catch(() => false)) {
    await allIn.click()
    await raise.click()
    return true
  }
  if (await call.isVisible().catch(() => false)) {
    await call.click()
    return true
  }
  return false
}

test('two players queue, play a match and see the result', async ({ browser }) => {
  const names = [`ann_${run}`, `ben_${run}`]
  const pages = await Promise.all(names.map(async () => (await browser.newContext()).newPage()))
  for (const [i, page] of pages.entries()) await signUp(page, names[i])

  for (const page of pages) {
    await page.getByRole('button', { name: 'Find a match' }).click()
  }
  for (const page of pages) {
    await expect(page).toHaveURL(/\/play$/)
    await expect(page.getByTestId('table')).toBeVisible()
  }

  // Each player sees their own two cards face up and the opponent's face down.
  for (const page of pages) {
    const me = page.getByTestId('seat-me')
    await expect(me.getByRole('img', { name: /of (spades|hearts|diamonds|clubs)/ })).toHaveCount(2, {
      timeout: 15_000,
    })
    const opp = page.getByTestId('seat-opponent')
    await expect(opp.getByRole('img', { name: 'face-down card' })).toHaveCount(2)
  }

  // Play: shove or call until someone wins.
  const result = pages.map((p) => p.getByTestId('result'))
  const deadline = Date.now() + 100_000
  while (!(await result[0].isVisible()) || !(await result[1].isVisible())) {
    if (Date.now() > deadline) throw new Error('match did not finish')
    let acted = false
    for (const page of pages) acted = (await shoveWhenAsked(page)) || acted
    if (!acted) await pages[0].waitForTimeout(200)
  }

  const headings = await Promise.all(result.map((r) => r.getByRole('heading').textContent()))
  expect(headings.sort()).toEqual(['You lost the match', 'You won the match'])
  for (const r of result) await expect(r).toContainText(/Rating 1500 → (1516|1484)/)

  // History shows the match from each side.
  const page = pages[0]
  await page.getByRole('button', { name: 'Back to the lobby' }).click()
  await page.getByRole('link', { name: 'Your matches', exact: true }).click()
  const row = page.locator('table.list tbody tr').first()
  await expect(row).toContainText(names[1])
  await row.getByRole('link').click()
  await page.locator('details summary').first().click()
  await expect(page.locator('details pre').first()).toContainText(`Dealt to ${names[0]}`)
  await expect(page.locator('details pre').first()).not.toContainText(`Dealt to ${names[1]}`)
})

test('the lobby is usable logged out and on a phone', async ({ browser }) => {
  const page = await (await browser.newContext({ viewport: { width: 390, height: 844 } })).newPage()
  await page.goto('/')
  await expect(page.getByRole('heading', { name: "Heads-up hold'em, ranked." })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Leaderboard' })).toBeVisible()
  // No horizontal scrolling.
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)
  expect(overflow).toBe(false)
  await page.goto('/history')
  await expect(page).toHaveURL(/\/login$/)
})

test('the information pages load and fit a phone', async ({ browser }) => {
  const page = await (await browser.newContext({ viewport: { width: 390, height: 844 } })).newPage()
  for (const [path, heading] of [
    ['/about', 'About Pokerladder'],
    ['/leaderboard', 'Leaderboard'],
    ['/privacy', 'Privacy notice'],
    ['/login', 'Log in'],
    ['/register', 'Create an account'],
  ]) {
    await page.goto(path)
    await expect(page.getByRole('heading', { name: heading, exact: true })).toBeVisible()
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)
    expect(overflow, `${path} scrolls sideways`).toBe(false)
  }
  // In-page links reach their section, even from the same page.
  await page.goto('/about')
  await page.locator('footer').getByRole('link', { name: 'Fair play' }).click()
  await expect(page.getByRole('heading', { name: 'Fair play' })).toBeInViewport()
})
