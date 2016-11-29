import numpy as np
from matplotlib import pyplot as plt

def make_rad_label(val):
    top = val/np.pi

    if top < 1:
        bottom = np.pi/val
        return r"$-\frac{\pi}{" + str(bottom) + "}$"
    else:
        return r"$-\frac{" + str(top) + "}{\pi}$"

def sine_wave_plot():
    fig = plt.figure()
    ax  = fig.add_subplot(111)
    ax.grid(True)

    ticklines = ax.get_xticklines() + ax.get_yticklines()
    gridlines = ax.get_xgridlines() + ax.get_ygridlines()

    for line in ticklines:
        line.set_linewidth(3)

    for line in gridlines:
        line.set_linestyle('-')

    unit   = np.pi/32
    x_tick = np.arange(0.0, 2.0*np.pi+unit, unit)
    x_pi   = x_tick/np.pi

    ax.plot(x_tick, np.sin(x_tick))
    unit   = np.pi/4
    x_tick = np.arange(0.0, 2.0*np.pi+unit, unit)

    x_label = [
        r"$0$",
        r"$\frac{\pi}{4}$",
        r"$\frac{\pi}{2}$",
        r"$\frac{3\pi}{4}$",
        r"$\pi$",
        r"$\frac{5\pi}{4}$",
        r"$\frac{3\pi}{2}$",
        r"$\frac{7\pi}{4}$",
        r"$2\pi$"]

    ax.set_xticks(x_tick)
    ax.set_xticklabels(x_label, fontsize=20)

    sample_rate = 0.75
    ss = np.arange(0.0, 2.0*np.pi+unit, sample_rate)
    for s in ss:
        plt.axvline(x=s, color='red')
        plt.plot(s, np.sin(s), 'ro', color='red')

    plt.xlim(0,2.0*np.pi)

    plt.show()

sine_wave_plot()
