---
layout: post
title: "Particle Simulator"
description: ""
category: 
tags: []
---

This post is a write up of a very small "day project" I did recently. The
project was a *very* naive particle motion simulation.

What is it
----------
I thought it would be cool to try and simulate simple masses moving around in
space as a programming exercise. So, I used some simple physics and made a
little toy.

Physics
-------
Particles are not charged. They are just particles with a mass, and some
velocity. Say particles $ p\_1 $ and $ p\_2 $ are floating around in space. Let's
also assume that $ p\_1 $ is moving with some velocity, but $ p\_2 $ is not
moving. If we want to figure out where $ p\_1 $ will be in $ t $ time, how would
we do that?

We know that the force on $p\_1$ will change its movement. Once we compute
that force, we can compute the acceleration the particle is undergoing, figure
out the new velocity of the particle, and use that to find the final location.

But, the problem is not quite that easy. As the particle moves, the force on the
particle will change, so its acceleration will change as it move, and it will
always be undergoing acceleration. So we can't just use $ \Delta x = vt $,
because $v$ is changing with time. We need to find some way to deal with the
continuous nature of the system.

But, for now, let us look at a very simple model. We have the two particles
mentioned above. We will compute the net force on $p\_1$ particle, then figure
out its acceleration (change in velocity), change the velocity, then figure out
its location in $t$ seconds. We must (possibly foolishly) assume that the affect
of acceleration due to the forces on the particle during this time period is
negligible.

The first thing we need is the force on $p\_1$. Since there are only two object
in our world, the only force that will exist is the force between $p\_1$ and
$p\_2$. This force is attractive, and is computed using the following:

$$ \textbf{F} = G \frac{m_1m_2}{\mid\textbf{r}^2\mid} \hat{\textbf{r}} $$

Where $ G $ is the Gravitational Constant, $ m\_1 $ and $ m\_2 $ are the masses
of $ p\_1 $ and $ p\_2 $ respectively, $ \textbf{r} $ is the vector between
$ p\_1 $ and $ p\_2 $, so $|\textbf{r}|$ (magnitudue of the vector $\textbf{r}$
is the distance between them. $ \hat{r} $ is the unit vector (has length of one,
that is what the litte hat means) describing the direction of the force
(towards $p\_2$).

So, now we have the force. We need to find the acceleration. That is easy,
because Newton made it easy:

$$ \textbf{F} = m\textbf{a} $$

Substituting the derivative of velocity for acceleration, we can see that

$$ \textbf{F} = m \frac{d\textbf{v}}{dt} $$

But, we aren't really going to use calculus to solve this (at least I think I'm
pretending we aren't for now), so lets go ahead an use
$\frac{\Delta v}{\Delta t}$ for acceleration for now:

$$ \textbf{F} = m \frac{\Delta \textbf{v}}{\Delta t} $$

Do some algebra to get:

$$ \Delta \textbf{v} = \frac{\textbf{F}}{m} \Delta t $$

So, we know $\textbf{F}$, let's go ahead and substitute that in, then cancel
unneeded masses:

$$ \Delta \textbf{v} = \frac{ G m\_2 \hat{\textbf{r}} } { | \textbf{r} |^2 }
\Delta t $$

So, now we can easily figure out how much the velocity of a particle should
change in $t$ seconds, given it only has a single force acting on it.

So, now we can compute the particles new velocity. Since we have assumed that no
acceleration happens during the time period.

###Quick Numeric Example of Something Kind of like the Above
Let $p\_1$ and $p\_2$ both be particles with mass 100 kg.

$p\_1$ is located at $(0,0)$, and $p\_2$ is at $(5,5)$.

$p\_1$ is not moving, $p\_2$ has a velocity of 10 m/s in y-direction, and 10 m/s
in x-direction.

We want to find where $p\_2$ will be in 0.1 seconds.

I am not going to use the formula we just derived in this example to avoid
dealing too much with vectors, but I will use the same process. This is the
process my simulator used.

####Step 1: Find the force
Find distance:
$$ |\textbf{r}|^2 = \sqrt{ (5-0)^2 + (5-0)^2 } = \sqrt{50} = 5\sqrt{2} $$

Find magnitude of $ \textbf{F} $:
$$ \textbf{F} = G \frac{(100)(100)}{(5\sqrt{2})^2} = G \frac{1000}{50} = 20G $$

To resolve components of $\textbf{F}$ we need an angle, since the triangle
representing the force is and isosceles right triangle, the angle with respect
to the horizontal is $ \frac{\pi}{4} $

Since the sine and cosine of $ \frac{\pi}{4} $ are both $\frac{\sqrt{2}}{2}$,
we can easily get the x and y components of the force.

$$ F\_x = \textbf{F} \frac{\sqrt{2}}{2} = \frac{20G\sqrt{2}}{2} = 10G\sqrt{2} $$
$$ F\_y = \textbf{F} \frac{\sqrt{2}}{2} = \frac{20G\sqrt{2}}{2} = 10G\sqrt{2} $$

####Step 2: Find $\Delta v$
We know that the velocity of the particle the instant $t=0$ is
$\langle 5,5 \rangle$. At $t=0.00001$ the velocity will be different, at
$t=0.000001$ the velocity will be different, because our particles are always 
accelerating. This is where we will use our (possibly foolish) assumption.
We assumed that $ \Delta t $ is so small that the
acceleration that occurs while $ \Delta t $ passes is so small we can ignore
it.

Because of our assumption, we can do this (In components):

$$ \Delta v\_x = \frac{F\_x}{m\_2}\Delta t = \frac{10G\sqrt{2}}{10}0.1 = G \frac{\sqrt{2}}{10} $$
$$ \Delta v\_y = \frac{F\_y}{m\_2}\Delta t = \frac{10G\sqrt{2}}{10}0.1 = G \frac{\sqrt{2}}{10} $$

So, the velocity of $p\_2$ will need to change that much due to the force.

####Step 3: Find a new velocity for $p\_2$

The initial velocity was 5 up and 5 right. This force is pulling us down and
left, so:

$$ V\_{x,new} = V\_x - \Delta V\_x = 10 - G \frac{\sqrt{2}}{10} \approx 9.99999$$
$$ V\_{y,new} = V\_y - \Delta V\_y = 10 - G \frac{\sqrt{2}}{10} \approx 9.99999$$

####Step 4: Find new position
Now, since we have assumed zero acceleration in the entire time frame from $t=0$
to $t=0.1$, we can use simple formulas to compute next locations.

$$ x\_{new} = x\_{old} + V\_{x,new} \Delta t = 5 + 9.99999(0.01) = 5.9999 $$
$$ y\_{new} = y\_{old} + V\_{y,new} \Delta t = 5 + 9.99999(0.01) = 5.9999 $$

So, the particle was at $(5,5)$, and it now is at $(5.9999,5.99999)$. Yay?

Each time-step in my simulation performed this calculation. So, it looks kind of
like this:

    function Timestep:
        calculate net force on each particle
        calclate change in velocity for each particle
        update each particle's velocity
        move each particle
        repeat

###The assumption
I am not sure if the technique I used to deal with the dynamic nature of the
system is a good or bad technique. On one hand, I can see bad behavior arising
very easily. Lets think about a correct solution for a moment.

Doing this correctly, with calculus, is described [here](http://hyperphysics.phy-astr.gsu.edu/hbase/avari.html),
but, I think that my technique is just an approximation of this technique. With
sufficiently small $ \Delta t$ for each time step, I think the approximation may
be decent. I am not sure, and have not actually worked much of this out with pen
and paper, so everything I just said, and all of my physics may be completely
wrong.

###More implementation notes
Other than not really doing the physics correctly, my simulation had some other
issues.

* If a particle flew off the top, it came back into the space on the bottom,
  but, it a particle were sitting still at the top, and a particle were
  sitting still at the bottom of the space, they would not feel force "over the
  edge." So, essentially, I defined a really weird shaped space with really
  really strange physical properties. If I were to try again, doing this
  correctly, I would probably let particles fly off the edge and cease to exist
  after doing so.
* No collision detection. If particles collided, nothing happened, they just
  flew threw each other. This is a serious problem. As particles get closer, the
  force between them increases substantially, then they collide in nature. My
  particles get really close, the force increases substantially, then they fly
  through each other and continue on their marry way.

###Conclusion
If I were to do this project again, I would certainly go about it differently. I
did not do a good job actually representing any real physics with my little toy,
but the goals of a short "day project" are fun and learning, both of which have
happened here. Most interesting to me is my apparent incompetence with physics.
This represents a major hole in my knowledge and is absolutely something I need
to work to fill. I guess I do know that I should have just gotten paper out and
started doing this correctly with calculus, but I would really like to be able
to determine how wrong what I just did really is. I still feel like a really
really small $ \Delta t $ would make my approximation pretty valid, but I do not
really know how to prove that. This is an issue with my understanding of
physics. Alternatively, I've fixated so much on this idea that I don't see what
is really going on. In a few months I will revisit this project and will likely
be able to tell how crazy I was when I did it (part of the reason I spent all
this time documenting my incorrect work).

If you for some reason read this far, thank you for reading, maybe drop
something in the comments?
