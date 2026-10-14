-- A tiny original sample schema (films and rentals) for driver tests and demos.
create table actor (actor_id serial primary key, first_name text not null, last_name text not null);
create table film (
  film_id serial primary key,
  title text not null,
  release_year int,
  rating numeric(3,1),
  added_at timestamptz default now(),
  meta jsonb
);
create table film_actor (actor_id int references actor, film_id int references film, primary key (actor_id, film_id));
create view film_list as select f.title, a.first_name || ' ' || a.last_name as actor from film f join film_actor fa using (film_id) join actor a using (actor_id);
create function film_count() returns bigint language sql as $$ select count(*) from film; $$;

insert into actor (first_name, last_name) values ('Ada', 'Stone'), ('Ben', 'Okafor'), ('Chen', 'Li');
insert into film (title, release_year, rating, meta) values
  ('The Quiet Harbor', 2019, 7.4, '{"genre":"drama"}'),
  ('Orbit of Glass', 2021, 8.1, '{"genre":"sci-fi"}'),
  ('Paper Lanterns', 2016, 6.9, null);
insert into film_actor values (1,1),(2,1),(2,2),(3,3);
