# Letgo

My brother in law buys and sells Lego sets. Checking the current price of each and every Lego set in the
portfolio takes much time if you do it manually. Instead Letgo reads an excel spreadsheet with the contents
of the portfolio, crawls Ebay for information about each piece, creates a summary of all current prices and
sends the result as an email.

## Security
There are 3 pieces of sensitive information which needed to be hidden:
1) The link to the portfolio
2) The email address where Letgo sends the results
3) The email credentials which are used to send the email

Obviously you shouldn't share those, so I was not able to commit them publicly in this repository. I also didn't
feel like introducing CLI arguments for that. Instead I decided to use a crate called "dotenv". That allows you
to create a .env file where you can store all those sensitive pieces of information to configure the application.
As long as you don't commit that, you are good to go.

## Crawling Ebay
Crawling Ebay is done by first reverse engineering how you need to structure the URL in order to get the results
you want. First of all I needed to get a special ID in order to later get the results. This was a simple as searching
"ebay.com" and reading a hidden input in the result HTML.
This ID needs to be supplied as the "\_trksid" parameter.
The next hurdle was that we needed to fake the user agent header. I think without setting the user agent header on our
crawl requests to "curl/7.71.1" ebay recognized that I tried to crawl the site and returned errors for my requests.
With that taken care of I was able to start getting the results I needed.
Using the create "scraper" it was pretty easy to get the pieces of information I needed from each request. Well, I thought
so at least... The program ran fine for a couple of months, but then ebay decided to obfuscate how they report the date
an item got sold. Instead of having the date in a single HTML element it was split in multiple elements with some
hidden elements in between. Although I was very easy to get around that this time, it shows that the program might need
regular patches when ebay decides to make it harder to crawl again.

## Performance
Since the portfolio might get quite large, it was necessary to keep performance in mind. 
On option would be to create a thread for each item in the portfolio and work on them in parallel. Since those threads
would be IO bound by the network they would be sleeping most of the time waiting for the result of a request.
Async IO shines for this task since we can work on those tasks concurrently without spawning too many threads.
Since async IO got stabilized in Rust it also get very convenient to work with.
The only real problem I had was that when too many tasks were spawned concurrently some of the tasks would time out.
Fortunatley it was enough to limit the number of concurrent tasks to the number of available CPU cores.
