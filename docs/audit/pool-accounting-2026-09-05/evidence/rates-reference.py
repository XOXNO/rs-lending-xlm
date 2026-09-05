from decimal import Decimal, getcontext
getcontext().prec = 90
R = 10**27
YEAR = 31_556_926_000

def half(x, y, d):
    return (x*y+d//2)//d

def compound(rate, dt):
    x = rate*dt
    total = R+x
    power = x
    for denominator in (2, 6, 24, 120, 720, 5040, 40320):
        power = half(power, x, R)
        total += half(power, 1, denominator)
    return total

for percent in (1, 5, 20, 100, 200):
    annual = percent*R//100
    rate = half(annual, 1, YEAR)
    delta = Decimal(compound(rate, YEAR))/R - (Decimal(annual)/R).exp()
    print(f"annual={percent}% annual_input_error={delta}")

rate, dt = 30_000_000_000, 1_000
factor = Decimal(compound(rate, dt))/R
exponent = Decimal(rate*dt)/R
error = factor - exponent.exp()
assert error > 0
print(f"exact_rate={rate} dt={dt} raw_factor_error={(error*R)}")

# Conservative direction of index/share conversion, over adversarial small domains.
for decimals in (3, 7, 18):
    units_to_ray = 10**(27-decimals)
    for index in (R//1000, R//1000+1, R-1, R, R+1, 10**36-1, 10**36):
        for amount in (1, 2, 3, 999, 1_000_000_000):
            value = amount*units_to_ray
            mint_supply = value*R//index
            mint_debt = (value*R+index-1)//index
            assert mint_supply*index <= value*R
            assert mint_debt*index >= value*R
print("conversion rational-direction checks: 105 combinations passed")
